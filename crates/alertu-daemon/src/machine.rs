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
use crate::transitions::{decide, Effect, Event};
use crate::webhook;
use alertu_common::config::Config;
use alertu_common::protocol::InputDeviceInfo;
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

/// Commands the IPC layer (and hotplug watcher) send into the machine.
pub enum Control {
    Toggle,
    Arm,
    Disarm,
    /// Re-enumerate input devices and re-open readers (device hotplug).
    Rescan,
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
    devices_tx: watch::Sender<Vec<InputDeviceInfo>>,
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
    pub devices_tx: watch::Sender<Vec<InputDeviceInfo>>,
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
            devices_tx: ch.devices_tx,
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
                self.apply(Event::Toggle).await;
            }
            InputSignal::Activity { source } => {
                if self.state == GuardState::Armed && self.grace_elapsed() {
                    info!(%source, "intrusion detected");
                    self.apply(Event::Intrusion).await;
                }
            }
        }
    }

    async fn on_lock(&mut self, locked: bool) {
        // We only care about an *unlock* while the alarm is engaged: a normal
        // password unlock disarms, racing the remote — whichever fires first
        // wins. `decide` emits no unlock effect here, since the session is
        // already unlocked.
        if !locked && self.state.is_active() {
            info!("session unlocked externally; disarming");
            self.apply(Event::SessionUnlocked).await;
        }
    }

    async fn on_control(&mut self, ctrl: Control) {
        match ctrl {
            Control::Toggle => self.apply(Event::Toggle).await,
            Control::Arm => self.apply(Event::ForceArm).await,
            Control::Disarm => self.apply(Event::ForceDisarm).await,
            Control::Rescan => {
                info!("device hotplug; rescanning input devices");
                self.spawn_readers();
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

        // Warning ticks are periodic within `Triggered`, not a state change, so
        // they live here rather than in the transition table.
        if let Some(warn_at) = self.next_warning {
            if now >= warn_at {
                self.sound.play_once(&self.cfg.warning_sound);
                self.next_warning = Some(now + WARNING_INTERVAL);
            }
        }

        if let Some(alarm_at) = self.alarm_deadline {
            if now >= alarm_at {
                self.apply(Event::CountdownElapsed).await;
            }
        }
    }

    // --- transition interpreter -------------------------------------------

    /// Run one event through the pure [`decide`] table and interpret its
    /// effects, then commit the new state.
    async fn apply(&mut self, event: Event) {
        let Some(transition) = decide(self.state, event) else {
            return;
        };
        for effect in transition.effects {
            self.run_effect(effect).await;
        }
        self.set_state(transition.next);
    }

    /// Perform a single side effect. This is the only place I/O happens.
    async fn run_effect(&mut self, effect: Effect) {
        match effect {
            Effect::LockSession => self.session.lock().await,
            Effect::UnlockSession => self.session.unlock().await,
            Effect::PlayBeep => self.sound.play_once(&self.cfg.beep_sound),
            Effect::StartSiren => self.sound.start_siren(&self.cfg.siren_sound),
            Effect::StopSiren => self.sound.stop_siren(),
            Effect::Snapshot => snapshot::capture_async(self.cfg.clone()),
            Effect::Webhook => webhook::fire(&self.cfg.alarm_webhook_url, GuardState::Alarm),
            Effect::StartGrace => {
                self.grace_until =
                    Some(Instant::now() + Duration::from_secs(self.cfg.grace_period_secs));
            }
            Effect::StartCountdown => {
                let now = Instant::now();
                self.alarm_deadline = Some(now + Duration::from_secs(self.cfg.alarm_delay_secs));
                self.next_warning = Some(now);
            }
            Effect::ClearTimers => {
                self.grace_until = None;
                self.alarm_deadline = None;
                self.next_warning = None;
            }
        }
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

        // Publish the fresh device list so subscribed GUIs refresh (incl. after
        // hotplug), even if the connected client never asks again.
        let _ = self
            .devices_tx
            .send(entries.iter().map(|e| e.to_info()).collect());

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
