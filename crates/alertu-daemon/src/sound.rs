//! Audio playback via an external player process.
//!
//! We shell out to whatever player is available (`paplay`, `pw-play`, `aplay`,
//! `ffplay`, `play`) instead of linking an audio library. This keeps the daemon
//! free of an ALSA/PulseAudio build dependency and mirrors the spec's choice to
//! shell out for webcam capture — at the cost of one subprocess per sound.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// A detected player: the program plus the fixed args that precede the file.
#[derive(Clone)]
struct Player {
    prog: &'static str,
    args: Vec<&'static str>,
}

/// Candidate players, in preference order.
fn detect_player() -> Option<Player> {
    const CANDIDATES: &[(&str, &[&str])] = &[
        ("paplay", &[]),
        ("pw-play", &[]),
        ("aplay", &["-q"]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("play", &["-q"]),
    ];
    for (prog, args) in CANDIDATES {
        if which(prog) {
            debug!(player = prog, "selected audio player");
            return Some(Player {
                prog,
                args: args.to_vec(),
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
    pub fn new() -> Self {
        SoundPlayer {
            player: detect_player(),
            siren: None,
        }
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
        cmd.stderr(std::process::Stdio::null());
        match cmd.spawn() {
            Ok(mut child) => {
                // Reap asynchronously so we don't leak zombies.
                tokio::spawn(async move {
                    let _ = child.wait().await;
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

impl Default for SoundPlayer {
    fn default() -> Self {
        Self::new()
    }
}

async fn siren_loop(player: Player, path: PathBuf, stop: Arc<AtomicBool>, notify: Arc<Notify>) {
    while !stop.load(Ordering::SeqCst) {
        let mut cmd = Command::new(player.prog);
        cmd.args(&player.args).arg(&path);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "failed to start siren playback");
                return;
            }
        };

        tokio::select! {
            _ = child.wait() => { /* finished one loop; play again */ }
            _ = notify.notified() => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return;
            }
        }
    }
}
