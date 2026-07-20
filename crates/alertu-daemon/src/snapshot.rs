//! Webcam capture via an external tool (`fswebcam` or `ffmpeg`).

use alertu_common::config::Config;
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use tokio::process::Command;
use tracing::{info, warn};

/// Capture a single still to `snapshot_dir`, named with a timestamp, and return
/// its path. Prefers `fswebcam`, falling back to `ffmpeg`.
pub async fn capture(cfg: &Config) -> Result<PathBuf> {
    tokio::fs::create_dir_all(&cfg.snapshot_dir)
        .await
        .with_context(|| format!("creating snapshot dir {}", cfg.snapshot_dir.display()))?;

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let out = cfg.snapshot_dir.join(format!("alertu-{stamp}.jpg"));

    let result = if which("fswebcam") {
        Command::new("fswebcam")
            .args(["-d", &cfg.camera_device, "-r", "1280x720", "--no-banner"])
            .arg(&out)
            .output()
            .await
    } else if which("ffmpeg") {
        Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "v4l2",
                "-i",
                &cfg.camera_device,
                "-frames:v",
                "1",
            ])
            .arg(&out)
            .output()
            .await
    } else {
        return Err(anyhow!(
            "no capture tool found (install fswebcam or ffmpeg)"
        ));
    };

    let output = result.context("running capture tool")?;
    if output.status.success() {
        info!(file = %out.display(), "snapshot captured");
        Ok(out)
    } else {
        Err(anyhow!(
            "capture failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Spawn a capture in the background so the state machine isn't blocked.
pub fn capture_async(cfg: Config) {
    tokio::spawn(async move {
        if let Err(e) = capture(&cfg).await {
            warn!(error = %e, "snapshot capture failed");
        }
    });
}

fn which(prog: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(prog).is_file())
}
