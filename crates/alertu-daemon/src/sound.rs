//! Audio playback via an external player process.
//!
//! We shell out to whatever player is available (`paplay`, `pw-play`, `aplay`,
//! `ffplay`, `play`) instead of linking an audio library. This keeps the daemon
//! free of an ALSA/PulseAudio build dependency and mirrors the spec's choice to
//! shell out for webcam capture — at the cost of one subprocess per sound.
//!
//! One consequence is worth stating plainly, because it decides whether the
//! alarm makes a sound at all: `paplay` and `pw-play` connect to a sound server
//! owned by a *user session*, and this daemon runs as a system user that has
//! none. They exit with "Host is down". Setting `alsa_device` in the config
//! switches playback to `aplay` against the hardware, which needs no session.
//!
//! Failures are logged rather than swallowed. That is not a detail: a player
//! that starts and then cannot reach a server leaves no other trace, so
//! discarding its stderr turns a mute alarm into an unexplainable one.

use alertu_common::config::Config;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::{ChildStderr, Command};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Consecutive failed playbacks after which the siren loop gives up.
const SIREN_MAX_FAILURES: u32 = 3;

/// Pause before retrying the siren after a failed playback.
const SIREN_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Cap on how much of a failed player's stderr reaches the log.
const MAX_STDERR_CHARS: usize = 300;

/// A detected player: the program plus the fixed args that precede the file.
#[derive(Clone)]
struct Player {
    prog: &'static str,
    args: Vec<String>,
}

/// Pick a player, honouring `cfg.alsa_device` when it is set.
fn detect_player(cfg: &Config) -> Option<Player> {
    const CANDIDATES: &[(&str, &[&str])] = &[
        ("paplay", &[]),
        ("pw-play", &[]),
        ("aplay", &["-q"]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("play", &["-q"]),
    ];

    // An explicit ALSA device means "bypass the session's sound server". Only
    // `aplay` takes `-D`, and it is the one candidate that reaches the hardware
    // without a per-user server to connect to, so an explicit device overrides
    // the preference order rather than being threaded through it.
    let device = cfg.alsa_device.trim();
    if !device.is_empty() {
        if which("aplay") {
            info!(device, "playing on the configured ALSA device");
            return Some(Player {
                prog: "aplay",
                args: vec!["-q".to_string(), "-D".to_string(), device.to_string()],
            });
        }
        warn!(
            device,
            "alsa_device is set but aplay was not found on PATH; falling back to \
             autodetection, which needs a session sound server the daemon may not have"
        );
    }

    for (prog, args) in CANDIDATES {
        if which(prog) {
            debug!(player = prog, "selected audio player");
            return Some(Player {
                prog,
                args: args.iter().map(|arg| arg.to_string()).collect(),
            });
        }
    }
    warn!("no audio player found (tried paplay/pw-play/aplay/ffplay/play); sound disabled");
    None
}

/// Whether `prog` is found on `PATH`.
fn which(prog: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(prog).is_file())
}

/// Plays one-shot sounds and manages a looping siren.
pub struct SoundPlayer {
    player: Option<Player>,
    siren: Option<SirenHandle>,
}

struct SirenHandle {
    stop: Arc<AtomicBool>,
    notify: Arc<Notify>,
    task: JoinHandle<()>,
}

impl SoundPlayer {
    pub fn new(cfg: &Config) -> Self {
        SoundPlayer {
            player: detect_player(cfg),
            siren: None,
        }
    }

    /// Re-pick the player after a live reconfigure, since `alsa_device` may have
    /// changed.
    ///
    /// A siren already sounding keeps the player it started with: re-detecting
    /// mid-alarm would mean cutting the siren to apply a setting, which is
    /// exactly the wrong trade-off for an alarm.
    pub fn reconfigure(&mut self, cfg: &Config) {
        self.player = detect_player(cfg);
    }

    /// Play `path` once, fire-and-forget. Missing files are logged, not fatal.
    pub fn play_once(&self, path: &Path) {
        let Some(player) = &self.player else { return };
        if !path.exists() {
            warn!(file = %path.display(), "sound file not found");
            return;
        }
        let mut cmd = Command::new(player.prog);
        cmd.args(&player.args).arg(path);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::piped());
        match cmd.spawn() {
            Ok(child) => {
                let prog = player.prog;
                let file = path.display().to_string();
                // Reap asynchronously so we don't leak zombies.
                tokio::spawn(async move {
                    match child.wait_with_output().await {
                        Ok(out) if out.status.success() => {}
                        Ok(out) => warn!(
                            player = prog,
                            file = %file,
                            code = ?out.status.code(),
                            stderr = %truncated(&out.stderr),
                            "sound playback failed"
                        ),
                        Err(e) => {
                            warn!(player = prog, error = %e, "cannot wait for the sound player")
                        }
                    }
                });
            }
            Err(e) => warn!(error = %e, "failed to play sound"),
        }
    }

    /// Start looping `path` until [`SoundPlayer::stop_siren`]. Idempotent-ish:
    /// an existing siren is stopped first.
    pub fn start_siren(&mut self, path: &Path) {
        self.stop_siren();
        let Some(player) = self.player.clone() else {
            return;
        };
        if !path.exists() {
            warn!(file = %path.display(), "siren sound file not found");
            return;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        let path = path.to_path_buf();
        let task = tokio::spawn(siren_loop(player, path, stop.clone(), notify.clone()));
        self.siren = Some(SirenHandle { stop, notify, task });
    }

    /// Stop the looping siren, if any.
    pub fn stop_siren(&mut self) {
        if let Some(handle) = self.siren.take() {
            handle.stop.store(true, Ordering::SeqCst);
            handle.notify.notify_waiters();
            handle.task.abort();
        }
    }
}

async fn siren_loop(player: Player, path: PathBuf, stop: Arc<AtomicBool>, notify: Arc<Notify>) {
    let mut failures = 0u32;

    while !stop.load(Ordering::SeqCst) {
        let mut cmd = Command::new(player.prog);
        cmd.args(&player.args).arg(&path);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "failed to start siren playback");
                return;
            }
        };
        // Taken before the select so the failure arm can still read it. The pipe
        // outlives the child, and a player's error message is orders of
        // magnitude smaller than the pipe buffer, so draining it after exit
        // cannot deadlock.
        let stderr = child.stderr.take();

        tokio::select! {
            status = child.wait() => {
                match status {
                    // Finished one loop; play again.
                    Ok(s) if s.success() => failures = 0,
                    Ok(s) => {
                        failures += 1;
                        let message = drain(stderr).await;
                        warn!(
                            player = player.prog,
                            code = ?s.code(),
                            stderr = %message,
                            failures,
                            "siren playback failed"
                        );
                        // Without the delay and the cap, a player that fails
                        // instantly — busy ALSA device, no sound server — gets
                        // respawned in a tight loop for as long as the alarm
                        // lasts, burning a core and flooding the journal.
                        if failures >= SIREN_MAX_FAILURES {
                            warn!(
                                "giving up on the siren after {SIREN_MAX_FAILURES} \
                                 consecutive failures"
                            );
                            return;
                        }
                        tokio::time::sleep(SIREN_RETRY_DELAY).await;
                    }
                    Err(e) => {
                        warn!(error = %e, "cannot wait for the siren player");
                        return;
                    }
                }
            }
            _ = notify.notified() => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return;
            }
        }
    }
}

/// Read whatever is left in a child's stderr pipe, for logging.
async fn drain(pipe: Option<ChildStderr>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = pipe.read_to_end(&mut buf).await;
    truncated(&buf)
}

/// A player's stderr, trimmed and capped to fit one log line.
fn truncated(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    match text.char_indices().nth(MAX_STDERR_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_alsa_device_is_passed_to_aplay() {
        let cfg = Config {
            alsa_device: "plughw:0,2".to_string(),
            ..Config::default()
        };
        // Skip where aplay is absent: the fallback path is what runs then, and
        // it is covered by `autodetection_ignores_an_empty_alsa_device`.
        if !which("aplay") {
            return;
        }
        let player = detect_player(&cfg).expect("aplay is on PATH");
        assert_eq!(player.prog, "aplay");
        assert!(player.args.iter().any(|arg| arg == "plughw:0,2"));
        assert!(player.args.iter().any(|arg| arg == "-D"));
    }

    #[test]
    fn autodetection_ignores_an_empty_alsa_device() {
        let cfg = Config::default();
        assert!(cfg.alsa_device.is_empty());
        // Whatever is installed, an empty device must never produce `-D`.
        if let Some(player) = detect_player(&cfg) {
            assert!(!player.args.iter().any(|arg| arg == "-D"));
        }
    }

    #[test]
    fn stderr_is_trimmed_and_capped() {
        assert_eq!(truncated(b"  boom  \n"), "boom");
        let long = "x".repeat(MAX_STDERR_CHARS + 50);
        let capped = truncated(long.as_bytes());
        assert!(capped.ends_with('…'));
        assert_eq!(capped.chars().count(), MAX_STDERR_CHARS + 1);
    }
}
