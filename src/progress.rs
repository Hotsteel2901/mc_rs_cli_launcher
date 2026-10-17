//! Optional, transport-neutral download progress reporting.
//!
//! The CLI renders progress with `indicatif`.  The TUI installs a sink here so
//! the same download code can feed the animated frontend without printing over
//! the alternate screen.  Child processes opt in through
//! `MC_LAUNCHER_TUI_PROGRESS` and emit one JSON object per line on stderr.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// A single transfer update. `id` is stable for the lifetime of one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferEvent {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub total: Option<u64>,
    pub done: bool,
}

type Sink = Box<dyn Fn(TransferEvent) + Send + Sync + 'static>;

static SINK: Mutex<Option<Sink>> = Mutex::new(None);
static CANCEL: AtomicBool = AtomicBool::new(false);
static CURRENT_JOB: AtomicU64 = AtomicU64::new(0);
static ENV_ENABLED: OnceLock<bool> = OnceLock::new();

fn env_enabled() -> bool {
    *ENV_ENABLED.get_or_init(|| {
        matches!(
            std::env::var("MC_LAUNCHER_TUI_PROGRESS").as_deref(),
            Ok("1") | Ok("true") | Ok("yes") | Ok("on")
        )
    })
}

/// Install (or replace) the in-process progress sink.
pub fn set_sink(sink: Sink) {
    if let Ok(mut slot) = SINK.lock() {
        *slot = Some(sink);
    }
}

/// Remove the in-process sink. Safe to call multiple times.
pub fn clear_sink() {
    if let Ok(mut slot) = SINK.lock() {
        *slot = None;
    }
}

/// True when a frontend is consuming progress, either in-process or via a pipe.
pub fn capture_active() -> bool {
    SINK.lock().map(|slot| slot.is_some()).unwrap_or(false) || env_enabled()
}

/// Reset the cooperative cancellation flag before a new job.
pub fn reset_cancel() {
    CANCEL.store(false, Ordering::SeqCst);
}

/// Ask the running transfer loop to stop at the next chunk boundary.
pub fn request_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

pub fn is_cancelled() -> bool {
    CANCEL.load(Ordering::SeqCst)
}

/// Tag in-process progress with the job that owns it. A value of 0 means no
/// job is active and late updates are dropped by the frontend.
pub fn set_job(id: u64) {
    CURRENT_JOB.store(id, Ordering::SeqCst);
}

pub fn current_job() -> u64 {
    CURRENT_JOB.load(Ordering::SeqCst)
}

/// Publish one transfer update.
pub fn transfer(id: &str, label: &str, bytes: u64, total: Option<u64>, done: bool) {
    let event = TransferEvent {
        id: id.to_string(),
        label: label.to_string(),
        bytes,
        total,
        done,
    };

    if let Ok(slot) = SINK.lock() {
        if let Some(sink) = slot.as_ref() {
            sink(event);
            return;
        }
    }

    if env_enabled() {
        let payload = serde_json::json!({
            "kind": "progress",
            "id": event.id,
            "label": event.label,
            "bytes": event.bytes,
            "total": event.total,
            "done": event.done,
        });
        // A write error here only means the parent disappeared; never abort a
        // download because progress could not be reported.
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "@@MC_TUI@@{payload}");
        let _ = stderr.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    #[test]
    fn current_job_tag_round_trips() {
        set_job(0);
        assert_eq!(current_job(), 0);
        set_job(42);
        assert_eq!(current_job(), 42);
        set_job(0);
    }

    #[test]
    fn sink_receives_events_and_can_be_removed() {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let seen_clone = seen.clone();
        set_sink(Box::new(move |event| {
            if let Ok(mut guard) = seen_clone.lock() {
                guard.push(event);
            }
        }));
        transfer("id", "Asset", 12, Some(20), false);
        assert_eq!(seen.lock().unwrap().len(), 1);
        clear_sink();
        transfer("id", "Asset", 20, Some(20), true);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
