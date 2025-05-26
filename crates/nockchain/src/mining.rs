use std::str::FromStr;
use std::sync::Arc;
use std::collections::VecDeque;
use std::path::PathBuf;

use kernels::miner::KERNEL;
use nockapp::kernel::checkpoint::JamPaths;
use nockapp::kernel::form::Kernel;
use nockapp::nockapp::driver::{IODriverFn, NockAppHandle, PokeResult};
use nockapp::nockapp::wire::Wire;
use nockapp::nockapp::NockAppError;
use nockapp::noun::slab::NounSlab;
use nockapp::noun::{AtomExt, NounExt};
use nockvm::noun::{Atom, D, T};
use nockvm::jets::hot::HotEntry;
use nockvm_macros::tas;
use tempfile::{tempdir, TempDir};
use tracing::{instrument, warn, info, debug};
use tokio::sync::{mpsc, Mutex};

pub enum MiningWire {
    Mined,
    Candidate,
    SetPubKey,
    Enable,
}

impl MiningWire {
    pub fn verb(&self) -> &'static str {
        match self {
            MiningWire::Mined => "mined",
            MiningWire::SetPubKey => "setpubkey",
            MiningWire::Candidate => "candidate",
            MiningWire::Enable => "enable",
        }
    }
}

impl Wire for MiningWire {
    const VERSION: u64 = 1;
    const SOURCE: &'static str = "miner";

    fn to_wire(&self) -> nockapp::wire::WireRepr {
        let tags = vec![self.verb().into()];
        nockapp::wire::WireRepr::new(MiningWire::SOURCE, MiningWire::VERSION, tags)
    }
}

#[derive(Debug, Clone)]
pub struct MiningKeyConfig {
    pub share: u64,
    pub m: u64,
    pub keys: Vec<String>,
}

impl FromStr for MiningKeyConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Expected format: "share,m:key1,key2,key3"
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err("Invalid format. Expected 'share,m:key1,key2,key3'".to_string());
        }

        let share_m: Vec<&str> = parts[0].split(',').collect();
        if share_m.len() != 2 {
            return Err("Invalid share,m format".to_string());
        }

        let share = share_m[0].parse::<u64>().map_err(|e| e.to_string())?;
        let m = share_m[1].parse::<u64>().map_err(|e| e.to_string())?;
        let keys: Vec<String> = parts[1].split(',').map(String::from).collect();

        Ok(MiningKeyConfig { share, m, keys })
    }
}

// Optimized mining state management
#[derive(Clone)]
struct OptimizedMiningState {
    hot_state: Arc<Vec<HotEntry>>,
    kernel_pool: Arc<Mutex<VecDeque<(Kernel, TempDir)>>>,
    snapshot_base_path: Arc<PathBuf>,
}

impl OptimizedMiningState {
    async fn new() -> Self {
        let hot_state = Arc::new(zkvm_jetpack::hot::produce_prover_hot_state());
        let kernel_pool = Arc::new(Mutex::new(VecDeque::new()));

        // Create base snapshot directory
        let snapshot_base = tempfile::tempdir()
            .expect("Failed to create base snapshot directory");
        let snapshot_base_path = Arc::new(
            snapshot_base.into_path()
        );

        // Pre-warm kernel pool with multiple instances
        let num_cores = num_cpus::get();
        let pool_size = (num_cores * 2).min(8); // Limit to reasonable number

        info!("Pre-warming kernel pool with {} instances", pool_size);

        {
            let mut pool = kernel_pool.lock().await;
            for i in 0..pool_size {
                match Self::create_kernel_instance(&hot_state, &snapshot_base_path, i).await {
                    Ok((kernel, temp_dir)) => {
                        pool.push_back((kernel, temp_dir));
                    }
                    Err(e) => {
                        warn!("Failed to create kernel instance {}: {:?}", i, e);
                    }
                }
            }

            info!("Kernel pool initialized with {} instances", pool.len());
        }

        Self {
            hot_state,
            kernel_pool,
            snapshot_base_path,
        }
    }

    async fn create_kernel_instance(
        hot_state: &[HotEntry],
        base_path: &PathBuf,
        instance_id: usize,
    ) -> Result<(Kernel, TempDir), String> {
        let snapshot_dir = tempfile::tempdir_in(base_path)
            .map_err(|e| format!("Failed to create temp dir: {}", e))?;
        let snapshot_path_buf = snapshot_dir.path().to_path_buf();
        let jam_paths = JamPaths::new(snapshot_dir.path());

        debug!("Creating kernel instance {} at {:?}", instance_id, snapshot_path_buf);

        let kernel = Kernel::load_with_hot_state_huge(
            snapshot_path_buf,
            jam_paths,
            KERNEL,
            hot_state,
            false,
        ).await.map_err(|e| format!("Failed to load kernel: {:?}", e))?;

        Ok((kernel, snapshot_dir))
    }

    async fn get_kernel(&self) -> Option<(Kernel, TempDir)> {
        let mut pool = self.kernel_pool.lock().await;
        pool.pop_front()
    }

    async fn return_kernel(&self, kernel: Kernel, temp_dir: TempDir) {
        let mut pool = self.kernel_pool.lock().await;
        if pool.len() < 8 { // Don't let pool grow too large
            pool.push_back((kernel, temp_dir));
        }
        // If pool is full, just drop the kernel (temp_dir will be cleaned up)
    }
}

pub fn create_mining_driver(
    mining_config: Option<Vec<MiningKeyConfig>>,
    mine: bool,
    init_complete_tx: Option<tokio::sync::oneshot::Sender<()>>,
) -> IODriverFn {
    Box::new(move |handle| {
        Box::pin(async move {
            let Some(configs) = mining_config else {
                enable_mining(&handle, false).await?;

                if let Some(tx) = init_complete_tx {
                    tx.send(()).map_err(|_| {
                        warn!("Could not send driver initialization for mining driver.");
                        NockAppError::OtherError
                    })?;
                }

                return Ok(());
            };

            if configs.len() == 1
                && configs[0].share == 1
                && configs[0].m == 1
                && configs[0].keys.len() == 1
            {
                set_mining_key(&handle, configs[0].keys[0].clone()).await?;
            } else {
                set_mining_key_advanced(&handle, configs).await?;
            }
            enable_mining(&handle, mine).await?;

            if let Some(tx) = init_complete_tx {
                tx.send(()).map_err(|_| {
                    warn!("Could not send driver initialization for mining driver.");
                    NockAppError::OtherError
                })?;
            }

            if !mine {
                return Ok(());
            }

            // Initialize optimized mining state
            info!("Initializing optimized mining state...");
            let mining_state = OptimizedMiningState::new().await;

            // Create channels for worker communication
            let (result_tx, mut result_rx) = mpsc::unbounded_channel::<NounSlab>();

            // Create multiple worker channels
            let num_workers = num_cpus::get().min(8); // Limit workers to reasonable number
            info!("Starting {} mining workers", num_workers);

            let mut worker_txs = Vec::new();
            for worker_id in 0..num_workers {
                let (worker_tx, worker_rx) = mpsc::unbounded_channel::<NounSlab>();
                worker_txs.push(worker_tx);

                let worker_result_tx = result_tx.clone();
                let worker_mining_state = mining_state.clone();

                tokio::spawn(async move {
                    optimized_mining_worker(
                        worker_id,
                        worker_rx,
                        worker_result_tx,
                        worker_mining_state,
                    ).await;
                });
            }

            // Drop the original result_tx to avoid keeping it alive
            drop(result_tx);

            let mut current_worker = 0;

            loop {
                tokio::select! {
                    effect_res = handle.next_effect() => {
                        let Ok(effect) = effect_res else {
                            warn!("Error receiving effect in mining driver: {effect_res:?}");
                            continue;
                        };
                        let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
                            drop(effect);
                            continue;
                        };

                        if effect_cell.head().eq_bytes("mine") {
                            let candidate_slab = {
                                let mut slab = NounSlab::new();
                                slab.copy_into(effect_cell.tail());
                                slab
                            };

                            // Send to next available worker (round-robin)
                            if let Some(worker_tx) = worker_txs.get(current_worker) {
                                if let Err(_) = worker_tx.send(candidate_slab) {
                                    warn!("Failed to send candidate to worker {}", current_worker);
                                }
                                current_worker = (current_worker + 1) % num_workers;
                            }
                        }
                    },
                    result = result_rx.recv() => {
                        if let Some(result_slab) = result {
                            handle
                                .poke(MiningWire::Mined.to_wire(), result_slab)
                                .await
                                .expect("Could not poke nockchain with mined PoW");
                        }
                    }
                }
            }
        })
    })
}

async fn optimized_mining_worker(
    worker_id: usize,
    mut candidate_rx: mpsc::UnboundedReceiver<NounSlab>,
    result_tx: mpsc::UnboundedSender<NounSlab>,
    mining_state: OptimizedMiningState,
) {
    info!("Mining worker {} started", worker_id);

    while let Some(candidate) = candidate_rx.recv().await {
        debug!("Worker {} processing candidate", worker_id);

        if let Some(result) = optimized_mining_attempt(candidate, &mining_state).await {
            if let Err(_) = result_tx.send(result) {
                warn!("Worker {} failed to send result", worker_id);
                break;
            }
        }
    }

    info!("Mining worker {} stopped", worker_id);
}

pub async fn optimized_mining_attempt(
    candidate: NounSlab,
    mining_state: &OptimizedMiningState,
) -> Option<NounSlab> {
    // Get kernel from pool or create new one
    let (kernel, temp_dir) = match mining_state.get_kernel().await {
        Some((kernel, temp_dir)) => (kernel, temp_dir),
        None => {
            debug!("Creating new kernel instance for mining attempt");
            match OptimizedMiningState::create_kernel_instance(
                &mining_state.hot_state,
                &mining_state.snapshot_base_path,
                0,
            ).await {
                Ok((kernel, temp_dir)) => (kernel, temp_dir),
                Err(e) => {
                    warn!("Failed to create kernel instance: {:?}", e);
                    return None;
                }
            }
        }
    };

    // Perform the actual mining computation
    let result = match kernel
        .poke(MiningWire::Candidate.to_wire(), candidate)
        .await
    {
        Ok(effects_slab) => {
            let mut result_slab = None;

            for effect in effects_slab.to_vec() {
                let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
                    drop(effect);
                    continue;
                };
                if effect_cell.head().eq_bytes("command") {
                    result_slab = Some(effect);
                    break;
                }
            }

            result_slab
        }
        Err(e) => {
            warn!("Mining attempt failed: {:?}", e);
            None
        }
    };

    // Return kernel to pool
    mining_state.return_kernel(kernel, temp_dir).await;

    result
}

// Legacy mining attempt function (kept for compatibility)
pub async fn mining_attempt(candidate: NounSlab, handle: NockAppHandle) -> () {
    let snapshot_dir =
        tokio::task::spawn_blocking(|| tempdir().expect("Failed to create temporary directory"))
            .await
            .expect("Failed to create temporary directory");
    let hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let snapshot_path_buf = snapshot_dir.path().to_path_buf();
    let jam_paths = JamPaths::new(snapshot_dir.path());
    // Spawns a new std::thread for this mining attempt
    let kernel =
        Kernel::load_with_hot_state_huge(snapshot_path_buf, jam_paths, KERNEL, &hot_state, false)
            .await
            .expect("Could not load mining kernel");
    let effects_slab = kernel
        .poke(MiningWire::Candidate.to_wire(), candidate)
        .await
        .expect("Could not poke mining kernel with candidate");
    for effect in effects_slab.to_vec() {
        let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
            drop(effect);
            continue;
        };
        if effect_cell.head().eq_bytes("command") {
            handle
                .poke(MiningWire::Mined.to_wire(), effect)
                .await
                .expect("Could not poke nockchain with mined PoW");
        }
    }
}

#[instrument(skip(handle, pubkey))]
async fn set_mining_key(
    handle: &NockAppHandle,
    pubkey: String,
) -> Result<PokeResult, NockAppError> {
    let mut set_mining_key_slab = NounSlab::new();
    let set_mining_key = Atom::from_value(&mut set_mining_key_slab, "set-mining-key")
        .expect("Failed to create set-mining-key atom");
    let pubkey_cord =
        Atom::from_value(&mut set_mining_key_slab, pubkey).expect("Failed to create pubkey atom");
    let set_mining_key_poke = T(
        &mut set_mining_key_slab,
        &[D(tas!(b"command")), set_mining_key.as_noun(), pubkey_cord.as_noun()],
    );
    set_mining_key_slab.set_root(set_mining_key_poke);

    handle
        .poke(MiningWire::SetPubKey.to_wire(), set_mining_key_slab)
        .await
}

async fn set_mining_key_advanced(
    handle: &NockAppHandle,
    configs: Vec<MiningKeyConfig>,
) -> Result<PokeResult, NockAppError> {
    let mut set_mining_key_slab = NounSlab::new();
    let set_mining_key_adv = Atom::from_value(&mut set_mining_key_slab, "set-mining-key-advanced")
        .expect("Failed to create set-mining-key-advanced atom");

    // Create the list of configs
    let mut configs_list = D(0);
    for config in configs {
        // Create the list of keys
        let mut keys_noun = D(0);
        for key in config.keys {
            let key_atom =
                Atom::from_value(&mut set_mining_key_slab, key).expect("Failed to create key atom");
            keys_noun = T(&mut set_mining_key_slab, &[key_atom.as_noun(), keys_noun]);
        }

        // Create the config tuple [share m keys]
        let config_tuple = T(
            &mut set_mining_key_slab,
            &[D(config.share), D(config.m), keys_noun],
        );

        configs_list = T(&mut set_mining_key_slab, &[config_tuple, configs_list]);
    }

    let set_mining_key_poke = T(
        &mut set_mining_key_slab,
        &[D(tas!(b"command")), set_mining_key_adv.as_noun(), configs_list],
    );
    set_mining_key_slab.set_root(set_mining_key_poke);

    handle
        .poke(MiningWire::SetPubKey.to_wire(), set_mining_key_slab)
        .await
}

//TODO add %set-mining-key-multisig poke
#[instrument(skip(handle))]
async fn enable_mining(handle: &NockAppHandle, enable: bool) -> Result<PokeResult, NockAppError> {
    let mut enable_mining_slab = NounSlab::new();
    let enable_mining = Atom::from_value(&mut enable_mining_slab, "enable-mining")
        .expect("Failed to create enable-mining atom");
    let enable_mining_poke = T(
        &mut enable_mining_slab,
        &[D(tas!(b"command")), enable_mining.as_noun(), D(if enable { 0 } else { 1 })],
    );
    enable_mining_slab.set_root(enable_mining_poke);
    handle
        .poke(MiningWire::Enable.to_wire(), enable_mining_slab)
        .await
}
