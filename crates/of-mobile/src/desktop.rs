//! Replaces the PC version's `desktop.rs` (same function). `alerts.rs` calls
//! [`notify`] from its own thread when an alert fires while the window is not
//! focused. Stage 0 only logs it; system notifications come with the
//! foreground service (plan, stage 3).

pub fn notify(app: &str, summary: &str, body: &str) {
    log::info!("{app}: alert: {summary} {body}");
}
