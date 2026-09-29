use super::rect::Rect;

/// Typed outcome of a single layout write across the `OsAdapter` seam.
/// The writer classifies; the caller matches — no substring sniffing of
/// error strings on either side. Human-readable detail survives only as
/// log payload, never as a match key.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteOutcome {
    /// The window is at the target. Also returned when the element goes
    /// unreadable mid-write: prod treats that as success and the next
    /// sweep heals any real displacement.
    Ok,
    /// Consecutive readbacks stopped moving (sub-pixel jitter at most), so
    /// further writes are futile — a busy app event loop ignoring writes.
    /// The caller backs off and retries on a timer.
    Pinned { target: Rect, actual: Rect },
    /// The write was accepted but never converged on the target
    /// (frame-vs-content shift, not-yet-resizable window). Retry on the
    /// next layout; never record as success.
    Drift { target: Rect, actual: Rect },
    /// The element is gone: cache miss plus refresh plus full discover all
    /// missed. The caller applies its gone-grace timer, not another poll —
    /// the writer already polled.
    Gone,
    /// Expected-transient AX contention (live resize, mid-flight element
    /// refresh). Carries the underlying message for logs only. Retry on
    /// the next layout.
    Transient(String),
}

impl WriteOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, WriteOutcome::Ok)
    }
}
