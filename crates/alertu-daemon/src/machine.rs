//! The guard state machine: `Idle → Armed → Triggered → Alarm → Idle`.
//!
//! A single task owns all mutable state and drives transitions from four
//! sources, multiplexed with `tokio::select!`:
//!
//! * input signals (remote toggle / intrusion activity),
//! * session lock-state changes (password unlock detection),
//! * IPC control commands (arm/disarm/toggle/get/set config),
//! * internal timers (siren countdown and warning ticks in `Triggered`).

use crate::devices;
use crate::input::{self, InputSignal};
use crate::session::SessionCtl;
use crate::snapshot;
use crate::sound::SoundPlayer;
use crate::webhook;
use alertu_common::config::Config;
use alertu_common::state::GuardState;
use evdev::KeyCode;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{error, info, warn};

/// Interval between discreet warning ticks during the `Triggered` countdown.
const WARNING_INTERVAL: Duration = Duration::from_secs(1);

/// Commands the IPC layer sends into the machine.
pub enum Control {
    Toggle,
    Arm,
    Disarm,
    GetConfig(oneshot::Sender<Config>),
    SetConfig(Box<Config>, oneshot::Sender<Result<(), String>>),
}

/// Owned, single-threaded state of the alarm.
pub struct Machine {
    state: GuardState,
    cfg: Config,
    cfg_path: PathBuf,

    session: SessionCtl,
    sound: SoundPlayer,

    state_tx: watch::Sender<GuardState>,
    sig_tx: mpsc::Sender<InputSignal>,
    sig_rx: mpsc::Receiver<InputSignal>,
    lock_rx: mpsc::Receiver<bool>,
    ctrl_rx: mpsc::Receiver<Control>,

    readers: Vec<JoinHandle<()>>,

    /// Set while armed; intrusion is ignored until now passes this instant.
    grace_until: Option<Instant>,
    /// In `Triggered`: when the siren fires.
    alarm_deadline: Option<Instant>,
    /// In `Triggered`: when to play the next warning tick.
    next_warning: Option<Instant>,
}

/// Wiring handed to [`Machine::new`].
pub struct Channels {
    pub state_tx: watch::Sender<GuardState>,
    pub sig_tx: mpsc::Sender<InputSignal>,
    pub sig_rx: mpsc::Receiver<InputSignal>,
    pub lock_rx: mpsc::Receiver<bool>,
    pub ctrl_rx: mpsc::Receiver<Control>,
}

impl Machine {
    pub fn new(
        cfg: Config,
        cfg_path: PathBuf,
        session: SessionCtl,
        sound: SoundPlayer,
        ch: Channels,
    ) -> Self {
        Machine {
            state: GuardState::Idle,
            cfg,
            cfg_path,
            session,
            sound,
            state_tx: ch.state_tx,
            sig_tx: ch.sig_tx,
            sig_rx: ch.sig_rx,
            lock_rx: ch.lock_rx,
            ctrl_rx: ch.ctrl_rx,
            readers: Vec::new(),
            grace_until: None,
            alarm_deadline: None,
            next_warning: None,
        }
    }

    /// Run the machine until all command/input channels close.
    pub async fn run(mut self) {
        self.spawn_readers();

        loop {
            let deadline = self.next_deadline();
            let mut tick = deadline.map(|d| Box::pin(tokio::time::sleep_until(d)));

            tokio::select! {
                maybe = self.sig_rx.recv() => match maybe {
                    Some(sig) => self.on_input(sig).await,
                    None => break,
                },
                maybe = self.lock_rx.recv() => {
                    if let Some(locked) = maybe {
                        self.on_lock(locked).await;
                    }
                },
                maybe = self.ctrl_rx.recv() => match maybe {
                    Some(ctrl) => self.on_control(ctrl).await,
                    None => break,
                },
                _ = async { match &mut tick {
                    Some(s) => s.await,
                    None => std::future::pending().await,
                } } => self.on_timer().await,
            }
        }
        info!("state machine stopped");
    }

    /// Earliest pending timer, if the current state has one.
    fn next_deadline(&self) -> Option<Instant> {
        match (self.alarm_deadline, self.next_warning) {
            (Some(a), Some(w)) => Some(a.min(w)),
            (a, w) => a.or(w),
        }
    }

    // --- event handlers ---------------------------------------------------

    async fn on_input(&mut self, sig: InputSignal) {
        match sig {
            InputSignal::RemoteToggle => {
                info!("remote toggle");
                self.toggle().await;
            }
            InputSignal::Activity { source } => {
                if self.state == GuardState::Armed && self.grace_elapsed() {
                    info!(%source, "intrusion detected");
                    self.enter_triggered();
                }
            }
        }
    }

    async fn on_lock(&mut self, locked: bool) {
        // We only care about an *unlock* while the alarm is engaged: a normal
        // password unlock disarms, racing the remote — whichever fires first
        // wins. The session is already unlocked, so we don't call loginctl.
        if !locked && self.state.is_active() {
            info!("session unlocked externally; disarming");
            self.reset_to_idle(false).await;
        }
    }

    async fn on_control(&mut self, ctrl: Control) {
        match ctrl {
            Control::Toggle => self.toggle().await,
            Control::Arm => {
                if self.state == GuardState::Idle {
                    self.arm().await;
                }
            }
            Control::Disarm => {
                if self.state.is_active() {
                    self.reset_to_idle(true).await;
                }
            }
            Control::GetConfig(tx) => {
                let _ = tx.send(self.cfg.clone());
            }
            Control::SetConfig(cfg, tx) => {
                let reply = self.apply_config(*cfg).await;
                let _ = tx.send(reply);
            }
        }
    }

    async fn on_timer(&mut self) {
        if self.state != GuardState::Triggered {
            return;
        }
        let now = Instant::now();

        if let Some(warn_at) = self.next_warning {
            if now >= warn_at {
                self.sound.play_once(&self.cfg.warning_sound);
                self.next_warning = Some(now + WARNING_INTERVAL);
            }
        }

        if let Some(alarm_at) = self.alarm_deadline {
            if now >= alarm_at {
                self.enter_alarm().await;
            }
        }
    }

    // --- transitions ------------------------------------------------------

    async fn toggle(&mut self) {
        match self.state {
            GuardState::Idle => self.arm().await,
            _ => self.reset_to_idle(true).await,
        }
    }

    async fn arm(&mut self) {
        self.session.lock().await;
        self.sound.play_once(&self.cfg.beep_sound);
        self.grace_until = Some(Instant::now() + Duration::from_secs(self.cfg.grace_period_secs));
        self.set_state(GuardState::Armed);
    }

    /// Return to `Idle`. When `do_unlock` is set we actively unlock the session
    /// (remote/forced disarm); when it's false the session was already unlocked
    /// by the user, so we only tear down alarm state.
    async fn reset_to_idle(&mut self, do_unlock: bool) {
        if do_unlock {
            self.session.unlock().await;
        }
        self.sound.stop_siren();
        self.sound.play_once(&self.cfg.beep_sound);
        self.grace_until = None;
        self.alarm_deadline = None;
        self.next_warning = None;
        self.set_state(GuardState::Idle);
    }

    fn enter_triggered(&mut self) {
        let now = Instant::now();
        self.alarm_deadline = Some(now + Duration::from_secs(self.cfg.alarm_delay_secs));
        self.next_warning = Some(now);
        self.set_state(GuardState::Triggered);
    }

    async fn enter_alarm(&mut self) {
        self.alarm_deadline = None;
        self.next_warning = None;
        self.set_state(GuardState::Alarm);

        self.sound.start_siren(&self.cfg.siren_sound);
        snapshot::capture_async(self.cfg.clone());
        webhook::fire(&self.cfg.alarm_webhook_url, GuardState::Alarm);
    }

    fn set_state(&mut self, new: GuardState) {
        if self.state != new {
            info!(from = %self.state, to = %new, "state change");
            self.state = new;
            let _ = self.state_tx.send(new);
        }
    }

    // --- helpers ----------------------------------------------------------

    fn grace_elapsed(&self) -> bool {
        self.grace_until.is_none_or(|until| Instant::now() >= until)
    }

    async fn apply_config(&mut self, cfg: Config) -> Result<(), String> {
        cfg.validate().map_err(|e| e.to_string())?;
        cfg.save(&self.cfg_path).map_err(|e| e.to_string())?;
        info!("configuration updated");
        self.cfg = cfg;
        self.session.update_from(&self.cfg).await;
        self.spawn_readers();
        Ok(())
    }

    /// (Re)spawn evdev reader tasks from the current config.
    fn spawn_readers(&mut self) {
        for handle in self.readers.drain(..) {
            handle.abort();
        }

        let entries = devices::list();
        let resolved = devices::resolve(&self.cfg, &entries);

        match &resolved.remote {
            Some(path) => info!(
                remote = %path.display(),
                name = resolved.remote_name.as_deref().unwrap_or("?"),
                "remote resolved"
            ),
            None => warn!(
                hint = %self.cfg.remote_name_hint,
                "no remote device resolved; remote toggle unavailable"
            ),
        }

        if let Some(remote) = resolved.remote {
            let keys = parse_toggle_keys(&self.cfg.toggle_keys);
            if keys.is_empty() {
                error!("no valid toggle keys parsed from config");
            } else {
                self.readers
                    .push(input::spawn_remote(remote, keys, self.sig_tx.clone()));
            }
        }

        for path in resolved.watch {
            self.readers
                .push(input::spawn_watch(path, self.sig_tx.clone()));
        }
    }
}

/// Parse configured key names (e.g. `"KEY_VOLUMEUP"`) into evdev key codes,
/// warning about (and skipping) any unknown names.
fn parse_toggle_keys(names: &[String]) -> Vec<KeyCode> {
    names
        .iter()
        .filter_map(|name| match KeyCode::from_str(name) {
            Ok(code) => Some(code),
            Err(_) => {
                warn!(key = %name, "unknown evdev key name in toggle_keys; ignoring");
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_keys_and_skips_unknown() {
        let keys = parse_toggle_keys(&[
            "KEY_VOLUMEUP".to_string(),
            "NOT_A_REAL_KEY".to_string(),
            "KEY_ENTER".to_string(),
        ]);
        assert_eq!(keys, vec![KeyCode::KEY_VOLUMEUP, KeyCode::KEY_ENTER]);
    }

    #[test]
    fn all_unknown_yields_empty() {
        assert!(parse_toggle_keys(&["ZzZ".to_string()]).is_empty());
    }
}
