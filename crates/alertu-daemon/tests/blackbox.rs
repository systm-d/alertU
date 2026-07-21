//! Black-box test: spawn the real `alertu-daemon` binary against a temporary
//! socket, with a fake `loginctl` on `PATH`, and drive it over IPC.
//!
//! This covers the wiring the pure `transitions` tests cannot reach: config
//! loading, the socket server, the control path, and the `LockedHint` monitor
//! that turns a password unlock into a disarm.

use alertu_common::config::Config;
use alertu_common::ipc_client::Client;
use alertu_common::state::GuardState;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Kills the daemon when the test ends, however it ends.
struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Harness {
    /// Kept alive so the temp dir outlives the daemon.
    _dir: tempfile::TempDir,
    socket: PathBuf,
    /// The shim reads this to answer `LockedHint`: "yes" or "no".
    locked_file: PathBuf,
    /// The shim appends every invocation's arguments here.
    log_file: PathBuf,
    _daemon: DaemonGuard,
}

impl Harness {
    fn connect(&self) -> Client {
        Client::connect(&self.socket).expect("connecting to the daemon socket")
    }

    fn shim_log(&self) -> String {
        fs::read_to_string(&self.log_file).unwrap_or_default()
    }

    /// Pretend the screen locker changed the session's locked state.
    fn set_locked(&self, locked: bool) {
        fs::write(&self.locked_file, if locked { "yes\n" } else { "no\n" }).unwrap();
    }
}

/// Poll `cond` every 50 ms until it returns true, or fail after `timeout`.
fn wait_until(timeout: Duration, what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out after {timeout:?} waiting for: {what}");
}

/// Write a `loginctl` replacement that logs its arguments and answers
/// `LockedHint` from a file the test controls.
fn write_shim(root: &Path, log: &Path, locked: &Path) -> PathBuf {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let shim = bin.join("loginctl");
    let script = format!(
        "#!/bin/sh\n\
         echo \"$@\" >> '{log}'\n\
         case \"$1\" in\n\
         show-seat) echo test-session ;;\n\
         list-sessions) echo 'test-session 1000 tester seat0' ;;\n\
         show-session) cat '{locked}' ;;\n\
         esac\n\
         exit 0\n",
        log = log.display(),
        locked = locked.display(),
    );
    fs::write(&shim, script).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// A config that touches nothing real. See the plan's safety constraints: the
/// `"auto"` defaults would make the daemon open the test machine's actual
/// keyboard and mouse, and a real `camera_device` would trigger the webcam.
fn test_config(root: &Path) -> Config {
    Config {
        remote_device: "/nonexistent/remote".to_string(),
        watch_devices: vec!["/nonexistent/watch".to_string()],
        grace_period_secs: 0,
        alarm_delay_secs: 1,
        beep_sound: PathBuf::from("/nonexistent/beep.wav"),
        warning_sound: PathBuf::from("/nonexistent/warning.wav"),
        siren_sound: PathBuf::from("/nonexistent/siren.wav"),
        snapshot_dir: root.join("snapshots"),
        camera_device: "/nonexistent/video".to_string(),
        session_id: "test-session".to_string(),
        alarm_webhook_url: String::new(),
        ..Config::default()
    }
}

fn start() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();

    let log_file = root.join("loginctl.log");
    let locked_file = root.join("locked");
    fs::write(&log_file, "").unwrap();
    fs::write(&locked_file, "no\n").unwrap();
    let bin = write_shim(&root, &log_file, &locked_file);

    let cfg_path = root.join("config.toml");
    test_config(&root).save(&cfg_path).unwrap();
    let socket = root.join("alertu.sock");

    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let child = Command::new(env!("CARGO_BIN_EXE_alertu-daemon"))
        .arg("--config")
        .arg(&cfg_path)
        .arg("--socket")
        .arg(&socket)
        .env("PATH", path_env)
        .env("RUST_LOG", "warn")
        .spawn()
        .expect("spawning alertu-daemon");

    let harness = Harness {
        _dir: dir,
        socket,
        locked_file,
        log_file,
        _daemon: DaemonGuard(child),
    };
    wait_until(
        Duration::from_secs(10),
        "the daemon socket to accept",
        || Client::connect(&harness.socket).is_ok(),
    );
    harness
}

#[test]
fn arming_locks_the_session_and_an_external_unlock_disarms() {
    let harness = start();
    let mut client = harness.connect();

    assert_eq!(client.get_state().unwrap(), GuardState::Idle);

    // `arm` is acknowledged as soon as the command is queued, so the state
    // change is observed by polling rather than assumed.
    client.arm().unwrap();
    wait_until(Duration::from_secs(5), "the daemon to reach Armed", || {
        client
            .get_state()
            .map(|s| s == GuardState::Armed)
            .unwrap_or(false)
    });
    wait_until(Duration::from_secs(5), "`loginctl lock-session`", || {
        harness.shim_log().contains("lock-session")
    });

    // The screen locker reports the session as locked. The monitor never emits
    // its first observation, so it must see "yes" before "no" is a change.
    harness.set_locked(true);
    std::thread::sleep(Duration::from_millis(1500));

    // The user types their password.
    harness.set_locked(false);
    wait_until(
        Duration::from_secs(5),
        "the external unlock to disarm",
        || {
            client
                .get_state()
                .map(|s| s == GuardState::Idle)
                .unwrap_or(false)
        },
    );
}

#[test]
fn toggle_arms_then_disarms_and_unlocks() {
    let harness = start();
    let mut client = harness.connect();

    client.toggle().unwrap();
    wait_until(Duration::from_secs(5), "toggle to arm", || {
        client
            .get_state()
            .map(|s| s == GuardState::Armed)
            .unwrap_or(false)
    });

    client.toggle().unwrap();
    wait_until(Duration::from_secs(5), "toggle to disarm", || {
        client
            .get_state()
            .map(|s| s == GuardState::Idle)
            .unwrap_or(false)
    });

    let log = harness.shim_log();
    assert!(log.contains("lock-session"), "shim log:\n{log}");
    assert!(log.contains("unlock-session"), "shim log:\n{log}");
}
