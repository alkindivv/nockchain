use std::time::Duration;
use tokio::time::interval;
use tracing::{info, warn};

use crate::mining::{MiningStats, OptimizedMiningState};

pub struct MiningStatsDisplay {
    stats: std::sync::Arc<MiningStats>,
    display_interval: Duration,
}

impl MiningStatsDisplay {
    pub fn new(stats: std::sync::Arc<MiningStats>, display_interval_secs: u64) -> Self {
        Self {
            stats,
            display_interval: Duration::from_secs(display_interval_secs),
        }
    }

    pub async fn start_display_loop(&self) {
        let mut interval = interval(self.display_interval);

        info!("🚀 Starting mining statistics display loop");

        loop {
            interval.tick().await;

            // Display main statistics
            let stats_summary = self.stats.get_stats_summary().await;
            info!("\n{}", stats_summary);

            // Display worker statistics every 5 intervals
            if self.should_display_worker_stats().await {
                let worker_stats = self.stats.get_worker_stats().await;
                info!("\n{}", worker_stats);
            }
        }
    }

    async fn should_display_worker_stats(&self) -> bool {
        // Display worker stats every 5 intervals (e.g., every 50 seconds if interval is 10s)
        let uptime = self.stats.start_time.elapsed();
        let intervals_passed = uptime.as_secs() / self.display_interval.as_secs();
        intervals_passed % 5 == 0
    }

    pub async fn display_once(&self) {
        let stats_summary = self.stats.get_stats_summary().await;
        let worker_stats = self.stats.get_worker_stats().await;

        println!("{}", stats_summary);
        println!("\n{}", worker_stats);
    }
}

// Function to create and start stats display task
pub fn spawn_stats_display_task(
    stats: std::sync::Arc<MiningStats>,
    display_interval_secs: u64,
) -> tokio::task::JoinHandle<()> {
    let display = MiningStatsDisplay::new(stats, display_interval_secs);

    tokio::spawn(async move {
        display.start_display_loop().await;
    })
}

// Function to display mining performance summary
pub async fn display_mining_performance_summary(stats: &MiningStats) {
    let uptime = stats.start_time.elapsed();
    let total_attempts = stats.total_attempts.load(std::sync::atomic::Ordering::Relaxed);
    let successful_blocks = stats.successful_blocks.load(std::sync::atomic::Ordering::Relaxed);
    let active_workers = stats.active_workers.load(std::sync::atomic::Ordering::Relaxed);

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

    info!(
        "🎯 MINING PERFORMANCE SUMMARY:\n\
        ⏱️  Runtime: {:.1}s | 🔨 Attempts: {} | ✅ Blocks: {} | 📊 Success: {:.3}%\n\
        ⚡ Rate: {:.2} attempts/s | 👷 Workers: {}",
        uptime.as_secs_f64(),
        total_attempts,
        successful_blocks,
        success_rate,
        attempts_per_second,
        active_workers
    );
}

// Function to log mining milestones
pub async fn log_mining_milestone(stats: &MiningStats, milestone_type: MiningMilestone) {
    match milestone_type {
        MiningMilestone::FirstBlock => {
            info!("🎉 MILESTONE: First block mined!");
        }
        MiningMilestone::BlockCount(count) => {
            info!("🏆 MILESTONE: {} blocks mined!", count);
        }
        MiningMilestone::AttemptCount(count) => {
            info!("💪 MILESTONE: {} mining attempts completed!", count);
        }
        MiningMilestone::UptimeHours(hours) => {
            info!("⏰ MILESTONE: {} hours of continuous mining!", hours);
        }
    }

    // Display current stats after milestone
    display_mining_performance_summary(stats).await;
}

#[derive(Debug, Clone)]
pub enum MiningMilestone {
    FirstBlock,
    BlockCount(u64),
    AttemptCount(u64),
    UptimeHours(u64),
}

// Function to check and log milestones
pub async fn check_and_log_milestones(stats: &MiningStats) {
    let total_attempts = stats.total_attempts.load(std::sync::atomic::Ordering::Relaxed);
    let successful_blocks = stats.successful_blocks.load(std::sync::atomic::Ordering::Relaxed);
    let uptime_hours = stats.start_time.elapsed().as_secs() / 3600;

    // Check for first block
    if successful_blocks == 1 {
        log_mining_milestone(stats, MiningMilestone::FirstBlock).await;
    }

    // Check for block milestones (every 10 blocks)
    if successful_blocks > 0 && successful_blocks % 10 == 0 {
        log_mining_milestone(stats, MiningMilestone::BlockCount(successful_blocks)).await;
    }

    // Check for attempt milestones (every 1000 attempts)
    if total_attempts > 0 && total_attempts % 1000 == 0 {
        log_mining_milestone(stats, MiningMilestone::AttemptCount(total_attempts)).await;
    }

    // Check for uptime milestones (every hour)
    if uptime_hours > 0 && uptime_hours % 1 == 0 {
        log_mining_milestone(stats, MiningMilestone::UptimeHours(uptime_hours)).await;
    }
}