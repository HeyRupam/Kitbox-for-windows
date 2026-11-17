//! Vocabulary for work in flight.
//!
//! The core doesn't own threads — the desktop shell and the Android host each
//! drive work their own way — but they agree on these types so progress looks
//! the same everywhere.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(pub u64);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "job-{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub job: JobId,
    /// What is happening right now, in the user's words: "Trying quality 68".
    pub stage: String,
    /// 0.0–1.0 when known. Solvers report fractional progress across probes.
    pub fraction: Option<f32>,
}

/// Somewhere to send progress. The desktop shell forwards these to the UI as
/// Tauri events; tests and the CLI use simpler sinks.
pub trait ProgressSink: Send + Sync {
    fn report(&self, progress: Progress);
}

/// Sink that throws everything away, for callers that don't care.
pub struct Silent;

impl ProgressSink for Silent {
    fn report(&self, _progress: Progress) {}
}

impl<F> ProgressSink for F
where
    F: Fn(Progress) + Send + Sync,
{
    fn report(&self, progress: Progress) {
        self(progress)
    }
}

/// Cooperative cancellation. Engines check this between probes, so a cancel
/// lands within one encode rather than at the end of the job.
#[derive(Debug, Clone, Default)]
pub struct Cancel(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}
