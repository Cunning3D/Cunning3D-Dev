use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task};
use futures_lite::future;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ComputeRecord {
    pub duration: Duration,
    pub start_time: Instant,
    pub end_time: Instant,
    pub thread_name: String,
}

/// Resource to store performance data
#[derive(Resource)]
pub struct PerformanceMonitor {
    // Enable periodic sysinfo sampling (CPU/mem). Off by default to avoid periodic hitches.
    pub sys_stats_enabled: bool,
    pub cpu_usage: f32,
    pub total_mem: u64,     // Bytes
    pub used_mem: u64,      // Bytes
    pub available_mem: u64, // Bytes

    // Node Profiling
    // Map NodeId -> Detailed Record
    pub node_cook_times: HashMap<Uuid, ComputeRecord>,

    // Optional: Cook count per frame to detect redundant cooks
    pub node_cook_counts: HashMap<Uuid, u32>,

    pub is_paused: bool,
}

impl PerformanceMonitor {
    pub fn clear(&mut self) {
        self.node_cook_times.clear();
        self.node_cook_counts.clear();
    }
}

impl Default for PerformanceMonitor {
    fn default() -> Self {
        Self {
            sys_stats_enabled: false,
            cpu_usage: 0.0,
            total_mem: 0,
            used_mem: 0,
            available_mem: 0,
            node_cook_times: HashMap::new(),
            node_cook_counts: HashMap::new(),
            is_paused: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PerfStatsSnapshot {
    cpu_usage: f32,
    total_mem: u64,
    used_mem: u64,
    available_mem: u64,
}

struct PerfStatsUpdateState {
    timer: Timer,
    task: Option<Task<PerfStatsSnapshot>>,
}

impl PerfStatsUpdateState {
    fn spawn_refresh_task(&mut self) {
        // IMPORTANT: sysinfo refresh can hitch on Windows; do it off the main thread.
        let pool = IoTaskPool::get();
        self.task = Some(pool.spawn(async move {
            let mut sys = System::new_with_specifics(
                RefreshKind::new()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything()),
            );
            sys.refresh_cpu();
            sys.refresh_memory();
            PerfStatsSnapshot {
                cpu_usage: sys.global_cpu_info().cpu_usage(),
                total_mem: sys.total_memory(),
                used_mem: sys.used_memory(),
                available_mem: sys.available_memory(),
            }
        }));
    }
}

impl Default for PerfStatsUpdateState {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(0.5, TimerMode::Repeating),
            task: None,
        }
    }
}

/// System to update hardware stats periodically (non-blocking)
pub fn update_performance_stats(
    mut monitor: ResMut<PerformanceMonitor>,
    time: Res<Time>,
    mut st: Local<PerfStatsUpdateState>,
) {
    if monitor.is_paused {
        st.task = None;
        return;
    }
    if !monitor.sys_stats_enabled {
        st.task = None;
        return;
    }

    // Poll async refresh task (non-blocking).
    if let Some(task) = &mut st.task {
        if let Some(snap) = future::block_on(future::poll_once(task)) {
            monitor.cpu_usage = snap.cpu_usage;
            monitor.total_mem = snap.total_mem;
            monitor.used_mem = snap.used_mem;
            monitor.available_mem = snap.available_mem;
            st.task = None;
        }
    }

    // Schedule refresh every 0.5 seconds (but do work off-thread).
    st.timer.tick(time.delta());
    if st.timer.just_finished() && st.task.is_none() {
        st.spawn_refresh_task();
    }

    // Reset per-frame counters (if any)
    // monitor.node_cook_counts.clear(); // We might want to clear this at Start of Frame
}

/// Plugin to setup profiling
pub struct ProfilingPlugin;

impl Plugin for ProfilingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PerformanceMonitor>()
            .add_systems(Update, update_performance_stats);
    }
}
