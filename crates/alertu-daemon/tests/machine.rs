//! In-process test of the state machine interpreter.
//!
//! `transitions.rs` proves the decision table; this proves the interpreter
//! wired to real timers — that an intrusion while armed escalates to the siren
//! after `alarm_delay_secs`, and that a disarm tears it all down.

use alertu_common::config::Config;
use alertu_common::protocol::InputDeviceInfo;
use alertu_common::state::GuardState;
use alertu_daemon::input::InputSignal;
use alertu_daemon::machine::{Channels, Control, Machine};
use alertu_daemon::perms::Privileges;
use alertu_daemon::session::SessionCtl;
use alertu_daemon::sound::SoundPlayer;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

/// A config that touches nothing real. See the plan's safety constraints:
/// `session_id` must not name a real logind session or `loginctl lock-session`
/// would lock the developer's screen, `camera_device` must not exist or
/// reaching `Alarm` would trigger the webcam, and `snapshot_dir` must live
/// under the caller's own tempdir so a real `Alarm` capture attempt (which
/// `create_dir_all`s it before checking for a capture tool) can't create a
/// directory outside the test's sandbox.
fn test_config(tmp: &Path) -> Config {
    Config {
        remote_device: "/nonexistent/remote".to_string(),
        watch_devices: vec!["/nonexistent/watch".to_string()],
        grace_period_secs: 0,
        // Keep this at 1s or higher, never lower: the `watch` channel coalesces,
        // so if `Triggered -> Alarm` fired before the test observed `Triggered`,
        // the receiver would only ever see `Alarm` and the `Triggered`
        // assertion would sit there until its 5-second timeout.
        alarm_delay_secs: 1,
        beep_sound: PathBuf::from("/nonexistent/beep.wav"),
        warning_sound: PathBuf::from("/nonexistent/warning.wav"),
        siren_sound: PathBuf::from("/nonexistent/siren.wav"),
        camera_device: "/nonexistent/video".to_string(),
        session_id: "test-session".to_string(),
        alarm_webhook_url: String::new(),
        snapshot_dir: tmp.join("snapshots"),
        ..Config::default()
    }
}

/// Wait until the observed state equals `want`, or fail after `timeout`.
async fn expect_state(rx: &mut watch::Receiver<GuardState>, want: GuardState, timeout: Duration) {
    let wait = async {
        loop {
            if *rx.borrow_and_update() == want {
                return;
            }
            rx.changed()
                .await
                .unwrap_or_else(|_| panic!("state channel closed while waiting for {want}"));
        }
    };
    if tokio::time::timeout(timeout, wait).await.is_err() {
        panic!("timed out after {timeout:?} waiting for state {want}");
    }
}

/// Handles for driving a machine that is already running.
struct Running {
    ctrl: mpsc::Sender<Control>,
    /// Injects input signals, standing in for the evdev reader tasks.
    input: mpsc::Sender<InputSignal>,
    state: watch::Receiver<GuardState>,
    /// Held only to keep the lock-state channel open. Dropping it would make
    /// `lock_rx.recv()` return `None` immediately and forever, and the
    /// machine's `select!` would spin on it.
    _lock: mpsc::Sender<bool>,
}

/// Wire up a `Machine` from `cfg` and spawn it.
async fn spawn_machine(cfg: Config, cfg_path: PathBuf) -> Running {
    let (sig_tx, sig_rx) = mpsc::channel(64);
    let (lock_tx, lock_rx) = mpsc::channel(16);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<Control>(32);
    let (state_tx, state_rx) = watch::channel(GuardState::Idle);
    let (devices_tx, _devices_rx) = watch::channel(Vec::<InputDeviceInfo>::new());

    // Our own handle for injecting intrusion; the machine keeps its clone for
    // the evdev readers it spawns (which all fail to open, harmlessly).
    let injector = sig_tx.clone();

    let session = SessionCtl::new(&cfg).await;
    let machine = Machine::new(
        cfg,
        cfg_path,
        session,
        SoundPlayer::new(),
        Privileges::default(),
        Channels {
            state_tx,
            devices_tx,
            sig_tx,
            sig_rx,
            lock_rx,
            ctrl_rx,
        },
    );
    tokio::spawn(machine.run());

    Running {
        ctrl: ctrl_tx,
        input: injector,
        state: state_rx,
        _lock: lock_tx,
    }
}

fn intrusion() -> InputSignal {
    InputSignal::Activity {
        source: "test device".to_string(),
    }
}

#[tokio::test]
async fn intrusion_while_armed_escalates_to_the_alarm_then_disarms() {
    let dir = tempfile::tempdir().unwrap();
    let mut run = spawn_machine(test_config(dir.path()), dir.path().join("config.toml")).await;

    run.ctrl.send(Control::Arm).await.unwrap();
    expect_state(&mut run.state, GuardState::Armed, Duration::from_secs(5)).await;

    run.input.send(intrusion()).await.unwrap();
    expect_state(
        &mut run.state,
        GuardState::Triggered,
        Duration::from_secs(5),
    )
    .await;

    // `alarm_delay_secs = 1`, so the siren fires about a second later.
    expect_state(&mut run.state, GuardState::Alarm, Duration::from_secs(5)).await;

    run.ctrl.send(Control::Disarm).await.unwrap();
    expect_state(&mut run.state, GuardState::Idle, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn activity_during_the_grace_period_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        grace_period_secs: 60,
        ..test_config(dir.path())
    };
    let mut run = spawn_machine(cfg, dir.path().join("config.toml")).await;

    run.ctrl.send(Control::Arm).await.unwrap();
    expect_state(&mut run.state, GuardState::Armed, Duration::from_secs(5)).await;

    run.input.send(intrusion()).await.unwrap();

    // Give the machine time to (not) react, then assert it stayed put.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(*run.state.borrow_and_update(), GuardState::Armed);

    // A `watch::Receiver` retains its last published value forever, so the
    // assertion above would pass identically if the machine task had panicked
    // or exited right after reaching `Armed`. Prove the event loop is still
    // alive and processing by round-tripping a command through it.
    let (cfg_tx, cfg_rx) = oneshot::channel();
    run.ctrl
        .send(Control::GetConfig(cfg_tx))
        .await
        .expect("machine event loop is dead: ctrl channel closed");
    let cfg = tokio::time::timeout(Duration::from_secs(5), cfg_rx)
        .await
        .expect("machine event loop is dead: GetConfig reply timed out")
        .expect("machine event loop is dead: GetConfig reply channel dropped");
    assert_eq!(cfg.grace_period_secs, 60);
}

/// Reaching `Alarm` must leave a snapshot directory the desktop user can read.
///
/// The capture itself fails — `camera_device` points at nothing, deliberately —
/// but `capture` creates the directory before invoking any tool, so the mode is
/// observable without going anywhere near a real webcam.
#[tokio::test]
async fn the_snapshot_directory_is_group_readable_not_world_readable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    // `test_config` already pins `snapshot_dir` inside the temp directory.
    let snapshots = cfg.snapshot_dir.clone();
    let mut run = spawn_machine(cfg, dir.path().join("config.toml")).await;

    run.ctrl.send(Control::Arm).await.unwrap();
    expect_state(&mut run.state, GuardState::Armed, Duration::from_secs(5)).await;
    run.input.send(intrusion()).await.unwrap();
    expect_state(&mut run.state, GuardState::Alarm, Duration::from_secs(10)).await;

    // The capture is spawned, so give it a moment to create the directory.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !snapshots.exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        snapshots.exists(),
        "the snapshot directory was never created"
    );

    let mode = std::fs::metadata(&snapshots).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o750, "snapshot dir mode was {mode:o}, expected 0750");
}
