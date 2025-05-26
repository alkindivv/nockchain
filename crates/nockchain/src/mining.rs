use std::str::FromStr;
use std::sync::Arc;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};

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

// Mining statistics structure
#[derive(Debug)]
pub struct MiningStats {
    pub start_time: Instant,
    pub total_attempts: Arc<AtomicU64>,
    pub successful_blocks: Arc<AtomicU64>,
    pub failed_attempts: Arc<AtomicU64>,
    pub active_workers: Arc<AtomicU32>,
    pub total_hash_rate: Arc<AtomicU64>,
    pub last_block_time: Arc<Mutex<Option<Instant>>>,
    pub average_attempt_time: Arc<Mutex<Duration>>,
    pub worker_stats: Arc<Mutex<Vec<WorkerStats>>>,
}

impl Clone for MiningStats {
    fn clone(&self) -> Self {
        Self {
            start_time: self.start_time,
            total_attempts: Arc::clone(&self.total_attempts),
            successful_blocks: Arc::clone(&self.successful_blocks),
            failed_attempts: Arc::clone(&self.failed_attempts),
            active_workers: Arc::clone(&self.active_workers),
            total_hash_rate: Arc::clone(&self.total_hash_rate),
            last_block_time: Arc::clone(&self.last_block_time),
            average_attempt_time: Arc::clone(&self.average_attempt_time),
            worker_stats: Arc::clone(&self.worker_stats),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerStats {
    pub worker_id: usize,
    pub attempts: u64,
    pub successes: u64,
    pub last_attempt_time: Option<Instant>,
    pub average_time_per_attempt: Duration,
}

impl MiningStats {
    pub fn new(num_workers: usize) -> Self {
        let worker_stats = (0..num_workers)
            .map(|id| WorkerStats {
                worker_id: id,
                attempts: 0,
                successes: 0,
                last_attempt_time: None,
                average_time_per_attempt: Duration::from_secs(0),
            })
            .collect();

        Self {
            start_time: Instant::now(),
            total_attempts: Arc::new(AtomicU64::new(0)),
            successful_blocks: Arc::new(AtomicU64::new(0)),
            failed_attempts: Arc::new(AtomicU64::new(0)),
            active_workers: Arc::new(AtomicU32::new(num_workers as u32)),
            total_hash_rate: Arc::new(AtomicU64::new(0)),
            last_block_time: Arc::new(Mutex::new(None)),
            average_attempt_time: Arc::new(Mutex::new(Duration::from_secs(0))),
            worker_stats: Arc::new(Mutex::new(worker_stats)),
        }
    }

    pub async fn record_attempt(&self, worker_id: usize, duration: Duration, success: bool) {
        self.total_attempts.fetch_add(1, Ordering::Relaxed);

        if success {
            self.successful_blocks.fetch_add(1, Ordering::Relaxed);
            let mut last_block = self.last_block_time.lock().await;
            *last_block = Some(Instant::now());
        } else {
            self.failed_attempts.fetch_add(1, Ordering::Relaxed);
        }

        // Update worker stats
        let mut workers = self.worker_stats.lock().await;
        if let Some(worker) = workers.get_mut(worker_id) {
            worker.attempts += 1;
            if success {
                worker.successes += 1;
            }
            worker.last_attempt_time = Some(Instant::now());

            // Update average time (simple moving average)
            let total_time = worker.average_time_per_attempt.as_nanos() as u64 * (worker.attempts - 1) + duration.as_nanos() as u64;
            worker.average_time_per_attempt = Duration::from_nanos(total_time / worker.attempts);
        }

        // Update global average
        let mut avg_time = self.average_attempt_time.lock().await;
        let total_attempts = self.total_attempts.load(Ordering::Relaxed);
        let total_time = avg_time.as_nanos() as u64 * (total_attempts - 1) + duration.as_nanos() as u64;
        *avg_time = Duration::from_nanos(total_time / total_attempts);
    }

    pub async fn get_stats_summary(&self) -> String {
        let uptime = self.start_time.elapsed();
        let total_attempts = self.total_attempts.load(Ordering::Relaxed);
        let successful_blocks = self.successful_blocks.load(Ordering::Relaxed);
        let failed_attempts = self.failed_attempts.load(Ordering::Relaxed);
        let active_workers = self.active_workers.load(Ordering::Relaxed);

        let success_rate = if total_attempts > 0 {
            (successful_blocks as f64 / total_attempts as f64) * 100.0
        } else {
            0.0
        };

        let attempts_per_second = if uptime.as_secs() > 0 {
            total_attempts as f64 / uptime.as_secs() as f64
        } else {
            0.0
        };

        let last_block = self.last_block_time.lock().await;
        let time_since_last_block = match *last_block {
            Some(time) => format!("{:.1}s ago", time.elapsed().as_secs_f64()),
            None => "Never".to_string(),
        };

        let avg_time = self.average_attempt_time.lock().await;

        format!(
            "🚀 NOCKCHAIN MINING STATS 🚀\n\
            ═══════════════════════════════════════\n\
            ⏱️  Uptime: {:.1}s\n\
            🔨 Total Attempts: {}\n\
            ✅ Successful Blocks: {}\n\
            ❌ Failed Attempts: {}\n\
            📊 Success Rate: {:.2}%\n\
            ⚡ Attempts/sec: {:.2}\n\
            👷 Active Workers: {}\n\
            🕒 Avg Attempt Time: {:.3}s\n\
            🏆 Last Block Found: {}\n\
            ═══════════════════════════════════════",
            uptime.as_secs_f64(),
            total_attempts,
            successful_blocks,
            failed_attempts,
            success_rate,
            attempts_per_second,
            active_workers,
            avg_time.as_secs_f64(),
            time_since_last_block
        )
    }

    pub async fn get_worker_stats(&self) -> String {
        let workers = self.worker_stats.lock().await;
        let mut result = String::from("👷 WORKER STATISTICS 👷\n");
        result.push_str("═══════════════════════════════════════\n");

        for worker in workers.iter() {
            let success_rate = if worker.attempts > 0 {
                (worker.successes as f64 / worker.attempts as f64) * 100.0
            } else {
                0.0
            };

            let last_activity = match worker.last_attempt_time {
                Some(time) => format!("{:.1}s ago", time.elapsed().as_secs_f64()),
                None => "Never".to_string(),
            };

            result.push_str(&format!(
                "Worker {}: {} attempts, {} blocks ({:.1}%), avg {:.3}s, last: {}\n",
                worker.worker_id,
                worker.attempts,
                worker.successes,
                success_rate,
                worker.average_time_per_attempt.as_secs_f64(),
                last_activity
            ));
        }

        result
    }
}

// Optimized mining state management
#[derive(Clone)]
pub struct OptimizedMiningState {
    hot_state: Arc<Vec<HotEntry>>,
    kernel_pool: Arc<Mutex<VecDeque<(Kernel, TempDir)>>>,
    snapshot_base_path: Arc<PathBuf>,
    stats: Arc<MiningStats>,
}

impl OptimizedMiningState {
    async fn new(num_workers: usize) -> Self {
        let hot_state = Arc::new(zkvm_jetpack::hot::produce_prover_hot_state());
        let kernel_pool = Arc::new(Mutex::new(VecDeque::new()));
        let stats = Arc::new(MiningStats::new(num_workers));

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
            stats,
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
            let num_workers = num_cpus::get().min(8); // Limit workers to reasonable number
            let mining_state = OptimizedMiningState::new(num_workers).await;

            // Create channels for worker communication
            let (result_tx, mut result_rx) = mpsc::unbounded_channel::<NounSlab>();

            // Create multiple worker channels
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

        let start_time = Instant::now();
        let result = optimized_mining_attempt(candidate, &mining_state).await;
        let duration = start_time.elapsed();

        let success = result.is_some();
        mining_state.stats.record_attempt(worker_id, duration, success).await;

        if let Some(result_slab) = result {
            info!("🎉 Worker {} found a block! Duration: {:.3}s", worker_id, duration.as_secs_f64());
            if let Err(_) = result_tx.send(result_slab) {
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
