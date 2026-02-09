//! App-wide background job system (no-hitch foundation).
//!
//! Design goals:
//! - Main thread never blocks on I/O / process wait / long CPU work
//! - Jobs run on Bevy task pools (IoTaskPool / AsyncComputeTaskPool)
//! - Progress + logs are streamed to UI
//! - Cancellation is best-effort (cooperative) and must never stall the main thread

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, IoTaskPool, Task};
use futures_lite::future;
use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

// -----------------------------
// Public types
// -----------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct JobProgress {
    pub fraction: f32, // 0..=1
    pub message: String,
}

impl Default for JobProgress {
    fn default() -> Self {
        Self {
            fraction: 0.0,
            message: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct JobLogLine {
    pub level: JobLogLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobLogLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone)]
pub struct JobError {
    pub message: String,
}

impl From<String> for JobError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for JobError {
    fn from(message: &str) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

/// Opaque output of a job. Concrete systems can downcast.
pub type JobOutput = Box<dyn std::any::Any + Send + Sync>;

pub struct JobContext {
    pub cancel: CancellationToken,
    pub progress: crossbeam_channel::Sender<JobProgress>,
    pub log: crossbeam_channel::Sender<JobLogLine>,
}

/// A runnable job spec (object-safe `FnOnce` equivalent).
pub trait JobRunnable: Send + Sync + 'static {
    fn title(&self) -> String;
    fn kind(&self) -> &'static str;
    fn pool(&self) -> JobPool;
    fn start(self: Box<Self>, cx: JobContext) -> Task<Result<JobOutput, JobError>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobPool {
    Io,
    Compute,
}

// -----------------------------
// Resource
// -----------------------------

#[derive(Resource, Default)]
pub struct AppJobs {
    next_id: u64,
    queue: VecDeque<(JobId, Box<dyn JobRunnable>)>,
    jobs: HashMap<JobId, JobEntry>,
    completed: VecDeque<JobId>,
}

impl AppJobs {
    pub fn enqueue(&mut self, job: Box<dyn JobRunnable>) -> JobId {
        let id = JobId(self.next_id.max(1));
        self.next_id = id.0.saturating_add(1);
        self.queue.push_back((id, job));
        id
    }

    pub fn cancel(&mut self, id: JobId) {
        if let Some(e) = self.jobs.get_mut(&id) {
            e.cancel.cancel();
            // State update happens when task observes cancellation or on poll.
        }
    }

    pub fn jobs(&self) -> &HashMap<JobId, JobEntry> {
        &self.jobs
    }

    pub fn completed_queue(&mut self) -> &mut VecDeque<JobId> {
        &mut self.completed
    }

    /// Take the output of a completed job (if any).
    pub fn take_output(&mut self, id: JobId) -> Option<JobOutput> {
        self.jobs.get_mut(&id).and_then(|e| e.output.take())
    }
}

pub struct JobEntry {
    pub id: JobId,
    pub title: String,
    pub kind: &'static str,
    pub state: JobState,
    pub progress: JobProgress,
    pub log: Vec<JobLogLine>,
    pub cancel: CancellationToken,
    pub output: Option<JobOutput>,
    task: Option<Task<Result<JobOutput, JobError>>>,
    rx_progress: crossbeam_channel::Receiver<JobProgress>,
    rx_log: crossbeam_channel::Receiver<JobLogLine>,
}

// -----------------------------
// Plugin + systems
// -----------------------------

pub struct AppJobsPlugin;

impl Plugin for AppJobsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AppJobs>()
            .add_systems(Update, (start_jobs_system, poll_jobs_system).chain());
    }
}

/// Start queued jobs (bounded concurrency).
fn start_jobs_system(mut jobs: ResMut<AppJobs>) {
    // Keep a conservative cap so background work doesn't starve foreground.
    const MAX_RUNNING: usize = 2;
    let running = jobs
        .jobs
        .values()
        .filter(|e| matches!(e.state, JobState::Running))
        .count();
    if running >= MAX_RUNNING {
        return;
    }

    // Start at most one per frame to keep scheduling deterministic.
    let Some((id, job)) = jobs.queue.pop_front() else {
        return;
    };

    let (tx_p, rx_p) = crossbeam_channel::unbounded::<JobProgress>();
    let (tx_l, rx_l) = crossbeam_channel::unbounded::<JobLogLine>();
    let cancel = CancellationToken::new();
    let title = job.title();
    let kind = job.kind();

    let cx = JobContext {
        cancel: cancel.clone(),
        progress: tx_p,
        log: tx_l,
    };

    let task = match job.pool() {
        JobPool::Io => IoTaskPool::get().spawn(job.start(cx)),
        JobPool::Compute => AsyncComputeTaskPool::get().spawn(job.start(cx)),
    };

    jobs.jobs.insert(
        id,
        JobEntry {
            id,
            title,
            kind,
            state: JobState::Running,
            progress: JobProgress::default(),
            log: Vec::new(),
            cancel,
            output: None,
            task: Some(task),
            rx_progress: rx_p,
            rx_log: rx_l,
        },
    );
}

/// Poll running jobs without blocking. Update progress/logs and move completed jobs to queue.
fn poll_jobs_system(mut jobs: ResMut<AppJobs>) {
    let ids: Vec<JobId> = jobs.jobs.keys().copied().collect();
    for id in ids {
        let Some(e) = jobs.jobs.get_mut(&id) else {
            continue;
        };

        // Drain progress/log streams.
        for p in e.rx_progress.try_iter() {
            e.progress = JobProgress {
                fraction: p.fraction.clamp(0.0, 1.0),
                message: p.message,
            };
        }
        for l in e.rx_log.try_iter() {
            e.log.push(l);
            if e.log.len() > 2000 {
                e.log.drain(0..(e.log.len() - 2000));
            }
        }

        if e.cancel.is_cancelled() && matches!(e.state, JobState::Running) {
            // Cooperative: job may still be winding down; reflect intent in UI immediately.
            e.state = JobState::Cancelled;
        }

        let Some(task) = &mut e.task else {
            continue;
        };
        if let Some(res) = future::block_on(future::poll_once(task)) {
            e.task = None;
            match res {
                Ok(out) => {
                    e.output = Some(out);
                    if matches!(e.state, JobState::Cancelled) {
                        // Keep Cancelled
                    } else {
                        e.state = JobState::Completed;
                    }
                }
                Err(err) => {
                    e.state = if matches!(e.state, JobState::Cancelled) {
                        JobState::Cancelled
                    } else {
                        JobState::Failed
                    };
                    e.log.push(JobLogLine {
                        level: JobLogLevel::Error,
                        message: err.message,
                    });
                }
            }
            jobs.completed.push_back(id);
        }
    }
}

