//! Optional HTTP hook fired on entering the `Alarm` state.
//!
//! This is the single forward-looking integration point kept in scope for v1
//! (e.g. a future mobile bridge). It POSTs a small JSON body via `curl` so the
//! daemon needs no HTTP/TLS client dependency.

use alertu_common::state::GuardState;
use tokio::process::Command;
use tracing::{debug, warn};

/// Fire the webhook if `url` is non-empty. Runs `curl` in the background with a
/// short timeout; failures are logged, never fatal.
pub fn fire(url: &str, state: GuardState) {
    if url.is_empty() {
        return;
    }
    if !which("curl") {
        warn!("alarm_webhook_url set but `curl` not found; skipping webhook");
        return;
    }

    let url = url.to_string();
    let stamp = chrono::Local::now().to_rfc3339();
    let body = format!(
        r#"{{"event":"alarm","state":"{}","timestamp":"{}"}}"#,
        state, stamp
    );

    tokio::spawn(async move {
        let result = Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--max-time",
                "5",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-d",
                &body,
                &url,
            ])
            .output()
            .await;
        match result {
            Ok(out) if out.status.success() => debug!(%url, "alarm webhook delivered"),
            Ok(out) => warn!(
                %url,
                stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                "alarm webhook returned an error"
            ),
            Err(e) => warn!(%url, error = %e, "failed to run curl for webhook"),
        }
    });
}

fn which(prog: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(prog).is_file())
}
