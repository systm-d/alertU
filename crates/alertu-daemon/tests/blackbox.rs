//! Black-box test: spawn the real `alertu-daemon` binary against a temporary
//! socket, with a fake `loginctl` on `PATH`, and drive it over IPC.
//!
//! This covers the wiring the pure `transitions` tests cannot reach: config
//! loading, the socket server, the control path, and the `LockedHint` monitor
//! that turns a password unlock into a disarm.

use alertu_common::config::Config;
use alertu_common::ipc_client::Client;
use alertu_common::protocol::Response;
use alertu_common::state::GuardState;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc::{self, RecvTimeoutError};
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
    /// Killed first: fields drop in declaration order, and the daemon must be
    /// gone before the temp dir holding its socket, config and shim goes away.
    _daemon: DaemonGuard,
    /// Kept alive so the temp dir outlives the daemon.
    _dir: tempfile::TempDir,
    socket: PathBuf,
    /// The shim reads this to answer `LockedHint`: "yes" or "no".
    locked_file: PathBuf,
    /// The shim appends every invocation's arguments here.
    log_file: PathBuf,
}

impl Harness {
    fn connect(&self) -> Client {
        Client::connect(&self.socket).expect("connecting to the daemon socket")
    }

    fn shim_log(&self) -> String {
        fs::read_to_string(&self.log_file).unwrap_or_default()
    }

    /// How many times the shim has been asked for the session's `LockedHint`.
    /// Used to prove the lock monitor actually polled, instead of sleeping for
    /// a duration guessed against its poll interval.
    fn show_session_calls(&self) -> usize {
        self.shim_log().matches("show-session").count()
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
        _daemon: DaemonGuard(child),
        _dir: dir,
        socket,
        locked_file,
        log_file,
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
    // Rather than sleeping for a guessed multiple of the 500 ms poll interval
    // (which a contended CI runner can outlast), wait for positive proof that
    // the monitor polled again: two more `show-session` calls, so at least one
    // whole poll started *after* the file said "yes".
    let polls_before = harness.show_session_calls();
    harness.set_locked(true);
    wait_until(
        Duration::from_secs(10),
        "the lock monitor to observe the locked session",
        || harness.show_session_calls() >= polls_before + 2,
    );

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

/// Regression: a connection that subscribes to a daemon which has already
/// published a state must not immediately receive a duplicate of the snapshot
/// its `Subscribe` reply just carried. The per-connection `watch::Receiver`s
/// are clones of long-lived ones that never mark anything seen, so without an
/// explicit `mark_unchanged` the first `changed()` resolves at once and
/// `status --watch` reports a phantom transition on every start-up.
#[test]
fn subscribing_after_a_state_change_pushes_nothing_until_the_next_transition() {
    let harness = start();
    let mut driver = harness.connect();

    // Make the daemon publish at least one state change *before* subscribing:
    // that is precisely what used to leave a fresh subscriber pre-flagged.
    driver.toggle().unwrap();
    wait_until(Duration::from_secs(5), "toggle to arm", || {
        driver
            .get_state()
            .map(|s| s == GuardState::Armed)
            .unwrap_or(false)
    });
    driver.toggle().unwrap();
    wait_until(Duration::from_secs(5), "toggle to disarm", || {
        driver
            .get_state()
            .map(|s| s == GuardState::Idle)
            .unwrap_or(false)
    });

    let mut watcher = harness.connect();
    assert_eq!(watcher.subscribe().unwrap(), GuardState::Idle);

    // `next_push` blocks with no timeout, so it is driven from a thread and
    // observed through a channel. `recv_timeout` then gives a bounded wait in
    // both directions with one primitive: the "nothing arrives" half fails
    // loudly instead of hanging, and the "a real transition does arrive" half
    // gets the pushed value itself. (A socket read timeout would need `Client`
    // to expose the underlying stream, and would degrade the second assertion
    // into matching on an error string.)
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(push) = watcher.next_push() {
            if tx.send(push).is_err() {
                break;
            }
        }
    });

    match rx.recv_timeout(Duration::from_secs(1)) {
        Err(RecvTimeoutError::Timeout) => {}
        Ok(push) => panic!(
            "a fresh subscriber was pushed {push:?} with no transition to report \
             (only a genuine /dev/input hotplug could legitimately push here)"
        ),
        Err(RecvTimeoutError::Disconnected) => panic!("the subscribed connection died"),
    }

    // The subscription still works: a genuine transition is pushed.
    driver.arm().unwrap();
    let push = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("no push after a real transition; the subscription is dead");
    assert_eq!(
        push,
        Response::StateChanged {
            state: GuardState::Armed
        }
    );
}

/// A daemon that cannot bind its control socket must fail loudly at startup.
///
/// Regression test for a silent failure found while running the daemon for real:
/// `serve` used to bind inside a spawned task whose `Result` was dropped, so an
/// over-long socket path left the daemon running happily with no IPC server and
/// not one word in the log — undiagnosable from the outside.
#[test]
fn an_unbindable_socket_aborts_startup_with_a_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();

    let cfg_path = root.join("config.toml");
    test_config(&root).save(&cfg_path).unwrap();

    // `sun_path` is 108 bytes; pad well past it so the bind cannot succeed.
    let socket = root.join(format!("{}.sock", "s".repeat(120)));
    assert!(
        socket.as_os_str().len() > 108,
        "test needs a path longer than sun_path"
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_alertu-daemon"))
        .arg("--config")
        .arg(&cfg_path)
        .arg("--socket")
        .arg(&socket)
        .env("RUST_LOG", "warn")
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawning alertu-daemon");

    // Poll rather than block on `wait`: the bug being guarded against left the
    // daemon running forever, and a hanging test is a far worse signal than a
    // failing one.
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("polling the daemon") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("daemon was still running 10s after an unbindable socket path");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    assert!(
        !status.success(),
        "daemon exited successfully despite an unbindable socket"
    );
    let mut buf = String::new();
    use std::io::Read as _;
    child
        .stderr
        .take()
        .expect("piped stderr")
        .read_to_string(&mut buf)
        .expect("reading stderr");
    let stderr = buf;
    assert!(
        stderr.contains("control socket"),
        "startup failure did not name the control socket; stderr was:\n{stderr}"
    );
}

/// A security flag whose value went missing must abort startup, not be quietly
/// dropped. `--socket-group` with nothing after it used to leave the socket on
/// the daemon's own group while the operator believed it was restricted; the
/// same parser also swallowed a following flag as the group name, so
/// `--socket-group --socket /p` lost the real socket path too.
#[test]
fn a_flag_with_no_value_aborts_startup() {
    for args in [
        vec!["--socket-group"],
        vec!["--socket-group", "--socket", "/tmp/alertu-never-bound.sock"],
        vec!["--config"],
        vec!["--socket"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_alertu-daemon"))
            .args(&args)
            .env("RUST_LOG", "warn")
            .output()
            .expect("spawning alertu-daemon");
        assert!(
            !out.status.success(),
            "daemon accepted {args:?}; it must refuse a flag with no value"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("expects a value"),
            "{args:?} did not explain itself; stderr was:\n{stderr}"
        );
    }
}

/// The control socket is a privilege boundary: group-accessible, never
/// world-accessible. A regression here silently exposes full alarm control
/// (disarm, read the webhook URL, redirect the helper programs) to every local
/// account.
///
/// The directory is asserted too, and not as an afterthought: the socket's mode
/// is only half the boundary. `bind` deliberately tightens the parent it owns so
/// that the window between `bind` and the socket's own `chmod` — during which the
/// socket carries whatever the umask produced — is not reachable by anyone
/// outside the group. Without this second assertion that whole mechanism is
/// untested: turning the directory into `0777` leaves every other test green.
#[test]
fn the_control_socket_is_not_world_accessible() {
    let harness = start();
    let mode = std::fs::metadata(&harness.socket)
        .expect("stat the socket")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o660, "socket mode was {mode:o}, expected 0660");

    let dir_mode = std::fs::metadata(harness.socket.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        dir_mode, 0o750,
        "socket dir mode was {dir_mode:o}, expected 0750"
    );
}
