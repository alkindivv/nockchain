use std::str::FromStr;
use std::time::{Duration, Instant};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kernels::miner::KERNEL;
use nockapp::kernel::checkpoint::JamPaths;
use nockapp::kernel::form::Kernel;
use nockapp::nockapp::driver::{IODriverFn, NockAppHandle, PokeResult};
use nockapp::nockapp::wire::Wire;
use nockapp::nockapp::NockAppError;
use nockapp::noun::slab::NounSlab;
use nockapp::noun::{AtomExt, NounExt};
use nockvm::noun::{Atom, D, T};
use nockvm_macros::tas;
use tempfile::tempdir;
use tracing::{instrument, warn, info, debug};

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

// Simple mining statistics
#[derive(Debug)]
pub struct SimpleMiningStats {
    pub start_time: Instant,
    pub total_attempts: Arc<AtomicU64>,
    pub successful_blocks: Arc<AtomicU64>,
}

impl SimpleMiningStats {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            total_attempts: Arc::new(AtomicU64::new(0)),
            successful_blocks: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn record_attempt(&self) {
        self.total_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_success(&self) {
        self.successful_blocks.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_stats(&self) -> (u64, u64, f64) {
        let attempts = self.total_attempts.load(Ordering::Relaxed);
        let successes = self.successful_blocks.load(Ordering::Relaxed);
        let uptime = self.start_time.elapsed().as_secs_f64();
        (attempts, successes, uptime)
    }
}

pub fn create_mining_driver(
    mining_config: Option<Vec<MiningKeyConfig>>,
    mine: bool,
    init_complete_tx: Option<tokio::sync::oneshot::Sender<()>>,
) -> IODriverFn {
    Box::new(move |mut handle| {
        Box::pin(async move {
            info!("🔧 Starting simple optimized mining driver...");

            let Some(configs) = mining_config else {
                info!("❌ No mining config provided, disabling mining");
                enable_mining(&handle, false).await?;

                if let Some(tx) = init_complete_tx {
                    tx.send(()).map_err(|_| {
                        warn!("Could not send driver initialization for mining driver.");
                        NockAppError::OtherError
                    })?;
                }

                return Ok(());
            };

            info!("🔑 Setting up mining key...");

            // Setup mining key with retry logic (but simpler than before)
            let mut setup_success = false;
            for attempt in 1..=3 {
                info!("🔄 Mining setup attempt {}/3", attempt);

                let key_result = if configs.len() == 1
                    && configs[0].share == 1
                    && configs[0].m == 1
                    && configs[0].keys.len() == 1
                {
                    set_mining_key(&handle, configs[0].keys[0].clone()).await
                } else {
                    set_mining_key_advanced(&handle, configs.clone()).await
                };

                if key_result.is_ok() {
                    info!("✅ Mining key set successfully on attempt {}", attempt);

                    // Try to enable mining
                    if enable_mining(&handle, mine).await.is_ok() {
                        info!("✅ Mining enabled successfully on attempt {}", attempt);
                        setup_success = true;
                        break;
                    } else {
                        warn!("⚠️ Failed to enable mining on attempt {}", attempt);
                    }
                } else {
                    warn!("⚠️ Failed to set mining key on attempt {}", attempt);
                }

                if attempt < 3 {
                    info!("⏳ Waiting 5s before retry...");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }

            if !setup_success {
                warn!("❌ Mining setup failed after 3 attempts, but continuing...");
            }

            if let Some(tx) = init_complete_tx {
                tx.send(()).map_err(|_| {
                    warn!("Could not send driver initialization for mining driver.");
                    NockAppError::OtherError
                })?;
                info!("📤 Mining driver initialization signal sent");
            }

            if !mine {
                info!("⏹️ Mining disabled, driver ready");
                return Ok(());
            }

            info!("🚀 Starting mining loop...");

            // Initialize simple stats
            let stats = Arc::new(SimpleMiningStats::new());
            let stats_clone = Arc::clone(&stats);

            // Spawn stats reporter
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    let (attempts, successes, uptime) = stats_clone.get_stats();
                    let rate = if uptime > 0.0 { attempts as f64 / uptime } else { 0.0 };

                    info!("📊 MINING STATS: {} attempts, {} blocks, {:.2} attempts/sec, {:.1}s uptime",
                          attempts, successes, rate, uptime);
                }
            });

            // Main mining loop (following original structure exactly)
            let mut next_attempt: Option<NounSlab> = None;
            let mut current_attempt: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

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
                            debug!("🎯 Received mining candidate");
                            let candidate_slab = {
                                let mut slab = NounSlab::new();
                                slab.copy_into(effect_cell.tail());
                                slab
                            };

                            if !current_attempt.is_empty() {
                                debug!("⏳ Mining in progress, queuing next candidate");
                                next_attempt = Some(candidate_slab);
                            } else {
                                debug!("🔨 Starting new mining attempt");
                                let (cur_handle, attempt_handle) = handle.dup();
                                handle = cur_handle;
                                let stats_ref = Arc::clone(&stats);
                                current_attempt.spawn(optimized_mining_attempt(candidate_slab, attempt_handle, stats_ref));
                            }
                        }
                    },
                    mining_attempt_res = current_attempt.join_next(), if !current_attempt.is_empty() => {
                        if let Some(Err(e)) = mining_attempt_res {
                            warn!("Error during mining attempt: {e:?}");
                        } else {
                            debug!("✅ Mining attempt completed");
                        }

                        let Some(candidate_slab) = next_attempt else {
                            continue;
                        };
                        next_attempt = None;
                        debug!("🔨 Starting queued mining attempt");
                        let (cur_handle, attempt_handle) = handle.dup();
                        handle = cur_handle;
                        let stats_ref = Arc::clone(&stats);
                        current_attempt.spawn(optimized_mining_attempt(candidate_slab, attempt_handle, stats_ref));
                    }
                }
            }
        })
    })
}

// Optimized mining attempt (keeping original structure but with improvements)
pub async fn optimized_mining_attempt(candidate: NounSlab, handle: NockAppHandle, stats: Arc<SimpleMiningStats>) -> () {
    let start_time = Instant::now();
    stats.record_attempt();

    // Create temporary directory
    let snapshot_dir = match tokio::task::spawn_blocking(|| tempdir()).await {
        Ok(Ok(dir)) => dir,
        Ok(Err(e)) => {
            warn!("Failed to create temporary directory: {:?}", e);
            return;
        }
        Err(e) => {
            warn!("Failed to spawn blocking task: {:?}", e);
            return;
        }
    };

    // Pre-compute hot state (this is expensive, so we want to reuse it)
    let hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let snapshot_path_buf = snapshot_dir.path().to_path_buf();
    let jam_paths = JamPaths::new(snapshot_dir.path());

    // Load kernel with hot state
    let kernel = match Kernel::load_with_hot_state_huge(snapshot_path_buf, jam_paths, KERNEL, &hot_state, false).await {
        Ok(kernel) => kernel,
        Err(e) => {
            warn!("Could not load mining kernel: {:?}", e);
            return;
        }
    };

    // Perform mining computation
    let effects_slab = match kernel.poke(MiningWire::Candidate.to_wire(), candidate).await {
        Ok(effects) => effects,
        Err(e) => {
            warn!("Could not poke mining kernel with candidate: {:?}", e);
            return;
        }
    };

    // Process results
    let mut found_block = false;
    for effect in effects_slab.to_vec() {
        let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
            drop(effect);
            continue;
        };
        if effect_cell.head().eq_bytes("command") {
            match handle.poke(MiningWire::Mined.to_wire(), effect).await {
                Ok(_) => {
                    let duration = start_time.elapsed();
                    info!("🎉 BLOCK FOUND! Mining attempt completed in {:.3}s", duration.as_secs_f64());
                    stats.record_success();
                    found_block = true;
                }
                Err(e) => {
                    warn!("Could not poke nockchain with mined PoW: {:?}", e);
                }
            }
        }
    }

    if !found_block {
        let duration = start_time.elapsed();
        debug!("⛏️ Mining attempt completed in {:.3}s (no block)", duration.as_secs_f64());
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