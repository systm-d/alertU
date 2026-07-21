# Lot C — Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn AlertU's control socket and its webcam evidence into a real privilege boundary, replace a 2-spawns-per-second poll loop with a D-Bus subscription, and make the tray's reconnection logic testable.

**Architecture:** A small `perms` module owns group resolution and mode setting for everything the daemon exposes. The socket and the snapshot directory share one group boundary rather than inventing two. Session lock observation moves to a zbus `PropertiesChanged` subscription with the existing poll loop kept as a fallback. The tray's session loop gains a two-method trait seam so it can be driven without D-Bus.

**Tech Stack:** Rust 2021, tokio, zbus 5, libc. Both new direct dependencies are already in the tree transitively — `zbus 5.18` via `ksni`, `libc 0.2` via several crates — so neither adds a new crate to the build.

**Spec:** `docs/superpowers/specs/2026-07-21-tooling-usability-hardening-design.md` (sections C1–C4)

## Global Constraints

- No new *system* library dependencies: no libasound/rodio, **no libdbus** (zbus is pure Rust), no GTK/Qt. Audio, camera capture and webhooks stay subprocess-based.
- `transitions.rs::decide` stays a pure function with no I/O. This plan does not touch it.
- No behaviour hardcoded for a specific remote model.
- IPC stays newline-delimited JSON with shared types in `alertu-common`.
- **The tray's four state-dependent icons must not change.** `icon_name()` and `icon_pixmap()` in `crates/alertu-gui/src/tray.rs` are out of bounds — the owner said so directly. Task 4 touches `main.rs` only.
- **`socket_group` must not become a `Config` field.** It is a command-line flag precisely so it cannot be changed over the very socket it protects.
- Before every commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` (zero warnings), `cargo test --workspace --all-features`.
- 81 tests pass at the start of this lot; none may be lost.
- Work on a branch `claude/lot-c`. Do not push.

## Safety Constraints For Tests

Unchanged from earlier lots, and still load-bearing. Any test or manual check that drives a daemon must pin:

- `session_id` to a non-existent session (`loginctl lock-session` would otherwise **lock the developer's screen**),
- `camera_device` to a non-existent path (`snapshot.rs::capture` does **not** check the device exists before invoking `fswebcam`/`ffmpeg`),
- `remote_device` and `watch_devices` to non-existent paths (the `"auto"` defaults open the host's **real keyboard and mouse**),
- `alarm_webhook_url` empty,
- `snapshot_dir` inside a temp directory.

Additionally, for this lot: **never run a test as root, and never assume the test user can `chown` to an arbitrary group.** A test may only chgrp to a group the running user already belongs to. Tests that would need privilege must be written so they skip with an explicit message rather than fail.

## Migration note this lot creates

After Task 2, the socket is `0660`. A desktop user who is not in the daemon's group **loses access to the tray, the settings window and `alertu-ctl`**. That is the point, but it is a breaking change for anyone already running AlertU. The README must say so plainly, with the one-line fix (`sudo usermod -aG alertu $USER`, then a new session).

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/alertu-daemon/src/perms.rs` (create) | Group resolution, chgrp, chmod — the only place that touches libc |
| `crates/alertu-daemon/src/ipc.rs` (modify) | Socket mode and group |
| `crates/alertu-daemon/src/snapshot.rs` (modify) | Snapshot directory and file permissions |
| `crates/alertu-daemon/src/machine.rs` (modify) | Carry the resolved group to the snapshot effect |
| `crates/alertu-daemon/src/session.rs` (modify) | D-Bus lock observation with a polling fallback |
| `crates/alertu-daemon/src/main.rs` (modify) | `--socket-group`, wiring |
| `crates/alertu-daemon/src/lib.rs` (modify) | Declare `perms` |
| `crates/alertu-gui/src/main.rs` (modify) | `TrayView` seam and its tests |
| `packaging/alertu-daemon.service` (modify) | `RuntimeDirectoryMode=0750` |
| `README.md` (modify) | Threat model, migration note |

---

### Task 1: The `perms` module

Everything that touches libc lives here, so the rest of the daemon stays free of `unsafe`.

**Files:**
- Create: `crates/alertu-daemon/src/perms.rs`
- Modify: `crates/alertu-daemon/src/lib.rs`, `crates/alertu-daemon/Cargo.toml`, root `Cargo.toml`

**Interfaces:**
- Consumes: nothing.
- Produces: `perms::resolve_gid(&str) -> Result<u32>`, `perms::chgrp(&Path, u32) -> Result<()>`, `perms::chmod(&Path, u32) -> Result<()>`, `perms::Privileges { group_gid: Option<u32> }`.

- [ ] **Step 1: Add the dependency**

In the root `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
libc = "0.2"
```

and in `crates/alertu-daemon/Cargo.toml`, to `[dependencies]`:

```toml
libc.workspace = true
```

`libc 0.2` is already in `Cargo.lock` transitively, so this adds no new crate to the build.

- [ ] **Step 2: Write the failing tests**

Create `crates/alertu-daemon/src/perms.rs` with only this test module for now:

```rust
//! Group ownership and file modes for everything the daemon exposes.
//!
//! The socket and the snapshot directory share one group boundary rather than
//! inventing two. This is the only module that touches libc, so the rest of the
//! daemon stays free of `unsafe`.

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn resolves_a_group_that_always_exists() {
        // `root` is gid 0 on every Linux system.
        assert_eq!(resolve_gid("root").unwrap(), 0);
    }

    #[test]
    fn an_unknown_group_is_an_error_naming_it() {
        let err = resolve_gid("definitely-not-a-real-group-9f3a").unwrap_err();
        assert!(
            err.to_string().contains("definitely-not-a-real-group-9f3a"),
            "error should name the group, got: {err}"
        );
    }

    #[test]
    fn a_group_name_with_a_nul_byte_is_rejected() {
        assert!(resolve_gid("bad\0name").is_err());
    }

    #[test]
    fn chmod_sets_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();

        chmod(&file, 0o640).unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640, "got {mode:o}");
    }

    #[test]
    fn chgrp_to_our_own_group_succeeds_and_leaves_the_owner_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();

        let before = std::fs::metadata(&file).unwrap();
        // Changing to the group we already have needs no privilege.
        let own_gid = before.gid();
        chgrp(&file, own_gid).unwrap();

        let after = std::fs::metadata(&file).unwrap();
        assert_eq!(after.gid(), own_gid);
        assert_eq!(after.uid(), before.uid(), "chgrp must not touch the owner");
    }

    #[test]
    fn chmod_on_a_missing_path_names_the_path() {
        let err = chmod(std::path::Path::new("/nonexistent/alertu/x"), 0o640).unwrap_err();
        assert!(err.to_string().contains("/nonexistent/alertu/x"), "got: {err}");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p alertu-daemon --lib perms`
Expected: FAIL — `cannot find function 'resolve_gid' in this scope`, and the same for `chgrp` and `chmod`.

- [ ] **Step 4: Implement**

Insert above the test module:

```rust
use anyhow::{anyhow, Context, Result};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Process-wide settings resolved from the command line rather than the config
/// file, so they cannot be changed over the socket they protect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Privileges {
    /// Group applied to the socket and to snapshots, when `--socket-group` was
    /// given. `None` means "leave the process's own group", which is already
    /// correct under the systemd unit.
    pub group_gid: Option<u32>,
}

/// Resolve a group name to its gid.
pub fn resolve_gid(name: &str) -> Result<u32> {
    let c_name = CString::new(name).with_context(|| format!("group name {name:?} contains a NUL byte"))?;

    // getgrnam_r fills a caller-provided buffer. 16 KiB is far past any real
    // /etc/group line, and we treat ERANGE as an error rather than growing:
    // a group that large is a misconfiguration, not a case to accommodate.
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::group = std::ptr::null_mut();

    // SAFETY: `c_name` is a valid NUL-terminated string, `grp` and `found` are
    // valid out-pointers we own, and `buf` is a writable buffer of the length
    // we pass. getgrnam_r writes only within those bounds.
    let rc = unsafe {
        libc::getgrnam_r(
            c_name.as_ptr(),
            &mut grp,
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };

    if rc != 0 {
        return Err(std::io::Error::from_raw_os_error(rc))
            .with_context(|| format!("looking up group {name}"));
    }
    if found.is_null() {
        return Err(anyhow!("no such group: {name}"));
    }
    Ok(grp.gr_gid)
}

/// Set the group owner of `path`, leaving the user owner untouched.
pub fn chgrp(path: &Path, gid: u32) -> Result<()> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .with_context(|| format!("path {} contains a NUL byte", path.display()))?;

    // SAFETY: `c_path` is a valid NUL-terminated path. `uid_t::MAX` is the
    // documented "leave unchanged" sentinel for chown(2).
    let rc = unsafe { libc::chown(c_path.as_ptr(), u32::MAX, gid) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("setting group {gid} on {}", path.display()));
    }
    Ok(())
}

/// Set the mode of `path`.
pub fn chmod(path: &Path, mode: u32) -> Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting mode {mode:o} on {}", path.display()))
}
```

Add `pub mod perms;` to `crates/alertu-daemon/src/lib.rs`, keeping the list alphabetical.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p alertu-daemon --lib perms`
Expected: PASS — 6 tests.

- [ ] **Step 6: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features`
Expected: clean, 87 tests.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(daemon): add a perms module for group ownership and file modes"
```

---

### Task 2: Socket hardening

**Files:**
- Modify: `crates/alertu-daemon/src/ipc.rs`, `crates/alertu-daemon/src/main.rs`, `crates/alertu-daemon/tests/blackbox.rs`, `packaging/alertu-daemon.service`, `README.md`

**Interfaces:**
- Consumes: `perms::{resolve_gid, chgrp, chmod, Privileges}` (Task 1).
- Produces: `ipc::bind(&Path, Privileges) -> Result<UnixListener>`; the daemon's `--socket-group <name>` flag.

- [ ] **Step 1: Write the failing test**

Add to `crates/alertu-daemon/tests/blackbox.rs`:

```rust
/// The control socket is a privilege boundary: group-accessible, never
/// world-accessible. A regression here silently exposes full alarm control
/// (disarm, read the webhook URL, redirect the helper programs) to every local
/// account.
#[test]
fn the_control_socket_is_not_world_accessible() {
    use std::os::unix::fs::PermissionsExt;

    let harness = start();
    let mode = std::fs::metadata(&harness.socket)
        .expect("stat the socket")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o660, "socket mode was {mode:o}, expected 0660");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p alertu-daemon --test blackbox the_control_socket`
Expected: FAIL — `socket mode was 666, expected 0660`.

- [ ] **Step 3: Take `Privileges` in `bind` and tighten the mode**

In `crates/alertu-daemon/src/ipc.rs`, change the signature and body of `bind`:

```rust
pub fn bind(socket_path: &Path, privileges: Privileges) -> Result<UnixListener> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating socket dir {}", parent.display()))?;
    }
    // Remove a stale socket from a previous run.
    if socket_path.exists() {
        std::fs::remove_file(socket_path)
            .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
    }

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("binding socket {}", socket_path.display()))?;
    perms::chmod(socket_path, 0o660)?;

    // With an explicit group, the parent directory must carry it too, or the
    // group cannot traverse into the socket and the flag is silently
    // inoperative. systemd recreates this directory on every start, so the
    // change does not persist.
    if let Some(gid) = privileges.group_gid {
        perms::chgrp(socket_path, gid)?;
        if let Some(parent) = socket_path.parent() {
            perms::chgrp(parent, gid)?;
            perms::chmod(parent, 0o750)?;
        }
    }

    info!(socket = %socket_path.display(), "IPC listening");
    Ok(listener)
}
```

Update the doc comment above `bind`: the socket is now `0o660`, reachable by the daemon's group rather than by everyone, and any failure here aborts startup.

Add `use crate::perms::{self, Privileges};` to the imports.

- [ ] **Step 4: Add the flag and wire it**

In `crates/alertu-daemon/src/main.rs`, add to `struct Args`:

```rust
    socket_group: Option<String>,
```

initialise it to `None` in `parse_args`, and add the arm:

```rust
            "--socket-group" => {
                if let Some(v) = it.next() {
                    socket_group = Some(v);
                }
            }
```

Update the `--help` text to:

```rust
                println!(
                    "alertu-daemon [--config <path>] [--socket <path>] [--socket-group <name>]\n\n\
                     Defaults:\n  config: {}\n  socket: {}\n  socket group: the daemon's own group\n\n\
                     The control socket is created 0660. Members of its group get full\n\
                     control of the alarm; treat membership as a privilege grant.",
                    Config::default_path().display(),
                    DEFAULT_SOCKET_PATH
                );
```

In `main`, resolve the group before anything else that could fail, so a typo is reported immediately:

```rust
    // Resolved up front: an unknown group must abort startup, never silently
    // leave a socket more permissive than the operator asked for.
    let privileges = Privileges {
        group_gid: match &args.socket_group {
            Some(name) => Some(
                perms::resolve_gid(name)
                    .with_context(|| format!("resolving --socket-group {name}"))?,
            ),
            None => None,
        },
    };
```

and pass it to `bind`:

```rust
    let listener = ipc::bind(&socket_path, privileges)
        .with_context(|| format!("binding the control socket {}", socket_path.display()))?;
```

Import `alertu_daemon::perms::{self, Privileges}`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p alertu-daemon --test blackbox`
Expected: PASS — 5 tests.

- [ ] **Step 6: Verify the hard-failure path by hand**

Run:

```bash
target/debug/alertu-daemon --socket /tmp/alertu-c.sock --socket-group definitely-not-a-group; echo "exit=$?"
```

Expected: a non-zero exit naming the group, and **no socket created** (`ls /tmp/alertu-c.sock` must fail). Paste the output into your report. Remove any socket you do create.

- [ ] **Step 7: Update the systemd unit**

In `packaging/alertu-daemon.service`, change:

```
RuntimeDirectoryMode=0755
```

to:

```
RuntimeDirectoryMode=0750
```

- [ ] **Step 8: Document the boundary and the migration**

In `README.md`, replace the threat-model paragraph, which still claims the socket is world-connectable, with an accurate one: the socket is `0660`, so access is limited to members of the daemon's group; that access is equivalent to full control — disarming, reading the config including the webhook URL, and `SetConfig`, which steers the paths handed to the helper programs. Group membership is a privilege grant, not a convenience.

Add a short **Upgrading** note near the install steps: users who ran an earlier version get a `0660` socket now and must join the daemon's group, `sudo usermod -aG alertu $USER` followed by a new login session, or the tray, the settings window and `alertu-ctl` will all fail to connect.

- [ ] **Step 9: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features`
Expected: clean, 88 tests.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat(daemon): make the control socket 0660 with an optional group"
```

---

### Task 3: Snapshot permissions

Alarm photographs currently inherit the capture subprocess's umask. Under the systemd unit they land `alertu:alertu 0640` and the desktop user cannot read them, silently.

**Files:**
- Modify: `crates/alertu-daemon/src/snapshot.rs`, `crates/alertu-daemon/src/machine.rs`, `crates/alertu-daemon/src/main.rs`, `crates/alertu-daemon/tests/machine.rs`

**Interfaces:**
- Consumes: `perms::{chgrp, chmod, Privileges}` (Task 1).
- Produces: `snapshot::capture(&Config, Privileges)`, `snapshot::capture_async(Config, Privileges)`; `Machine::new` gains a `Privileges` parameter.

- [ ] **Step 1: Write the failing test**

Add to `crates/alertu-daemon/tests/machine.rs`:

```rust
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
    assert!(snapshots.exists(), "the snapshot directory was never created");

    let mode = std::fs::metadata(&snapshots).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o750, "snapshot dir mode was {mode:o}, expected 0750");
}
```

`test_config(tmp: &Path)` already exists in this file with that signature and already pins `snapshot_dir` under `tmp`, along with every other safety value — no change to it is needed.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p alertu-daemon --test machine the_snapshot_directory`
Expected: FAIL — the mode is whatever the ambient umask produced (`0775` or `0755`), not `0750`.

- [ ] **Step 3: Apply permissions in `capture`**

In `crates/alertu-daemon/src/snapshot.rs`, change `capture` and `capture_async` to take `Privileges`, and apply the boundary:

```rust
pub async fn capture(cfg: &Config, privileges: Privileges) -> Result<PathBuf> {
    tokio::fs::create_dir_all(&cfg.snapshot_dir)
        .await
        .with_context(|| format!("creating snapshot dir {}", cfg.snapshot_dir.display()))?;

    // Alarm photographs are of whoever is at the machine — including the owner.
    // They get the same group boundary as the control socket, and deliberately
    // not 0644: world-readable webcam stills would be a privacy regression.
    perms::chmod(&cfg.snapshot_dir, 0o750)?;
    if let Some(gid) = privileges.group_gid {
        perms::chgrp(&cfg.snapshot_dir, gid)?;
    }

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let out = cfg.snapshot_dir.join(format!("alertu-{stamp}.jpg"));

    // … the existing fswebcam / ffmpeg selection is unchanged …

    let output = result.context("running capture tool")?;
    if output.status.success() {
        // The capture tool created the file under its own umask, so fix the
        // mode afterwards rather than trying to control the child's umask.
        perms::chmod(&out, 0o640)?;
        if let Some(gid) = privileges.group_gid {
            perms::chgrp(&out, gid)?;
        }
        info!(file = %out.display(), "snapshot captured");
        Ok(out)
    } else {
        Err(anyhow!(
            "capture failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub fn capture_async(cfg: Config, privileges: Privileges) {
    tokio::spawn(async move {
        if let Err(e) = capture(&cfg, privileges).await {
            warn!(error = %e, "snapshot capture failed");
        }
    });
}
```

Add `use crate::perms::{self, Privileges};`.

- [ ] **Step 4: Carry `Privileges` through the machine**

In `crates/alertu-daemon/src/machine.rs`, add a field to `Machine`:

```rust
    privileges: Privileges,
```

add the parameter to `Machine::new` after `sound`, store it, and change the snapshot effect:

```rust
            Effect::Snapshot => snapshot::capture_async(self.cfg.clone(), self.privileges),
```

In `crates/alertu-daemon/src/main.rs`, pass `privileges` to `Machine::new`. In `crates/alertu-daemon/tests/machine.rs`, pass `Privileges::default()` from `spawn_machine`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p alertu-daemon --test machine`
Expected: PASS — 3 tests.

- [ ] **Step 6: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features`
Expected: clean, 89 tests.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(daemon): give alarm snapshots the same group boundary as the socket"
```

---

### Task 4: A testable tray session loop

`alertu-gui` has one test. The two behaviours that carry real safety content — that a reconnection replays `Subscribe`/`GetConfig`/`ListDevices` in order, and that requests queued while offline are dropped rather than replayed — are unverified, because `ksni` always registers with a live `StatusNotifierWatcher` and offers no headless mode.

**Files:**
- Modify: `crates/alertu-gui/src/main.rs`, `crates/alertu-gui/Cargo.toml`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `trait TrayView { async fn set_connected(&self, bool); async fn apply(&self, Response); }`; `run_session` takes `&impl TrayView`.

**Hard constraint:** do not touch `crates/alertu-gui/src/tray.rs` at all in this task.

- [ ] **Step 1: Add the dev-dependency**

In `crates/alertu-gui/Cargo.toml`:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Introduce the seam**

In `crates/alertu-gui/src/main.rs`, add above `run_session`:

```rust
/// What a session does to the UI.
///
/// A seam, not a redesign. It exists so `run_session` can be driven in tests
/// against a recording fake: `ksni::TrayMethods::spawn` always registers with a
/// live `StatusNotifierWatcher`, so there is no way to exercise the reconnection
/// logic through the real tray.
trait TrayView {
    async fn set_connected(&self, connected: bool);
    async fn apply(&self, response: Response);
}

impl TrayView for ksni::Handle<AlertuTray> {
    async fn set_connected(&self, connected: bool) {
        self.update(move |t| t.connected = connected).await;
    }

    async fn apply(&self, response: Response) {
        apply_response(self, response).await;
    }
}
```

Change `run_session`'s third parameter from `handle: &ksni::Handle<AlertuTray>` to `view: &impl TrayView`, and inside it replace:

- `handle.update(|t| t.connected = true).await;` with `view.set_connected(true).await;`
- `apply_response(handle, resp).await` with `view.apply(resp).await`

In `main`, replace `handle.update(|t| t.connected = false).await;` with `handle.set_connected(false).await;` and keep passing `&handle` to `run_session`.

- [ ] **Step 3: Write the failing tests**

Add to `crates/alertu-gui/src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::io::AsyncBufReadExt as _;
    use tokio::net::UnixListener;

    #[test]
    fn backoff_doubles_and_sticks_at_the_cap() {
        let mut d = INITIAL_BACKOFF;
        let mut seen = vec![d];
        for _ in 0..8 {
            d = next_backoff(d);
            seen.push(d);
        }
        assert_eq!(
            seen,
            vec![
                Duration::from_millis(250),
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(10),
                Duration::from_secs(10),
                Duration::from_secs(10),
            ]
        );
    }

    /// Records what a session did to the UI, standing in for the real tray.
    #[derive(Default)]
    struct RecordingView {
        connected: Mutex<Vec<bool>>,
        applied: Mutex<Vec<Response>>,
    }

    impl TrayView for RecordingView {
        async fn set_connected(&self, connected: bool) {
            self.connected.lock().unwrap().push(connected);
        }
        async fn apply(&self, response: Response) {
            self.applied.lock().unwrap().push(response);
        }
    }

    /// A fake daemon that reads `expect` request lines, optionally answers, then
    /// hangs up. Returns the requests it saw.
    async fn fake_daemon(
        listener: UnixListener,
        expect: usize,
        replies: Vec<&'static str>,
    ) -> Vec<Request> {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();

        let mut seen = Vec::new();
        for reply in replies {
            use tokio::io::AsyncWriteExt as _;
            write_half.write_all(reply.as_bytes()).await.unwrap();
            write_half.write_all(b"\n").await.unwrap();
        }
        while seen.len() < expect {
            match lines.next_line().await.unwrap() {
                Some(l) if l.trim().is_empty() => {}
                Some(l) => seen.push(serde_json::from_str(&l).unwrap()),
                None => break,
            }
        }
        seen
    }

    #[tokio::test]
    async fn a_session_resynchronises_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("s.sock");
        let listener = UnixListener::bind(&socket).unwrap();

        let (_tx, mut rx) = mpsc::unbounded_channel::<Request>();
        let view = RecordingView::default();

        let daemon = tokio::spawn(fake_daemon(listener, 3, vec![]));
        let session = run_session(&socket, &mut rx, &view);
        let (seen, outcome) = tokio::join!(daemon, session);

        assert_eq!(
            seen.unwrap(),
            vec![Request::Subscribe, Request::GetConfig, Request::ListDevices],
            "a reconnection must resynchronise state, config and devices, in that order"
        );
        assert!(outcome.is_ok());
        assert_eq!(*view.connected.lock().unwrap(), vec![true]);
    }

    #[tokio::test]
    async fn requests_queued_while_disconnected_are_dropped_not_replayed() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("s.sock");
        let listener = UnixListener::bind(&socket).unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel::<Request>();
        // Queued while offline: replaying this against a live alarm is exactly
        // the hazard the drain exists to prevent.
        tx.send(Request::Arm).unwrap();
        tx.send(Request::Toggle).unwrap();

        let view = RecordingView::default();
        let daemon = tokio::spawn(fake_daemon(listener, 3, vec![]));
        let session = run_session(&socket, &mut rx, &view);
        let (seen, _) = tokio::join!(daemon, session);

        let seen = seen.unwrap();
        assert!(
            !seen.contains(&Request::Arm) && !seen.contains(&Request::Toggle),
            "stale requests must never reach the daemon, saw: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_pushed_response_reaches_the_view() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("s.sock");
        let listener = UnixListener::bind(&socket).unwrap();

        let (_tx, mut rx) = mpsc::unbounded_channel::<Request>();
        let view = RecordingView::default();

        let daemon = tokio::spawn(fake_daemon(
            listener,
            3,
            vec![r#"{"event":"state_changed","state":"armed"}"#],
        ));
        let session = run_session(&socket, &mut rx, &view);
        let (_, outcome) = tokio::join!(daemon, session);

        assert!(outcome.unwrap().got_response, "a response must be recorded");
        assert_eq!(
            *view.applied.lock().unwrap(),
            vec![Response::StateChanged {
                state: alertu_common::state::GuardState::Armed
            }]
        );
    }
}
```

`Request` and `Response` derive `PartialEq` and `Debug` already, which these assertions rely on. `alertu-gui` will need `alertu_common::state::GuardState` in scope for the last test.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p alertu-gui`
Expected: PASS — 4 tests. If a test hangs, the fake daemon and `run_session` are deadlocked: the fake must hang up (drop the listener's stream by returning) so `run_session` sees EOF and returns.

- [ ] **Step 5: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features`
Expected: clean, 92 tests.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "test(gui): cover session resync and the stale-request drop"
```

---

### Task 5: logind over D-Bus

Replaces a 500 ms `loginctl` poll — two subprocess spawns per second, indefinitely — with a property subscription, keeping the poll as a fallback.

**Files:**
- Modify: `crates/alertu-daemon/src/session.rs`, `crates/alertu-daemon/src/main.rs`, `crates/alertu-daemon/Cargo.toml`, root `Cargo.toml`, `README.md`

**Interfaces:**
- Consumes: `SessionCtl`.
- Produces: `session::watch(SessionCtl, mpsc::Sender<bool>, Duration)`, replacing the direct call to `session::monitor` in `main`.

- [ ] **Step 1: Add the dependency**

Root `Cargo.toml`, `[workspace.dependencies]`:

```toml
zbus = { version = "5", default-features = false, features = ["tokio"] }
```

`crates/alertu-daemon/Cargo.toml`, `[dependencies]`:

```toml
zbus.workspace = true
```

`zbus 5.18` is already in `Cargo.lock` via `ksni`, so this adds no new crate. `default-features = false` with the `tokio` feature keeps it off async-io and on the runtime the daemon already uses — verify no second reactor is pulled in with `cargo tree -p alertu-daemon | grep -c async-io`.

- [ ] **Step 2: Implement the subscription with its fallback**

In `crates/alertu-daemon/src/session.rs`, add:

```rust
/// Observe the session's lock state, preferring D-Bus and falling back to
/// polling.
///
/// A precision that shapes this: logind's `Lock`/`Unlock` signals are *requests*
/// addressed to the screen locker, not notifications that anything changed. The
/// observable state change is `PropertiesChanged` on `LockedHint`, which is the
/// real-time equivalent of what `monitor` polls.
///
/// Any failure — no system bus, an unknown session, a dropped stream — falls
/// back to the poll loop rather than leaving the daemon blind, because unlock
/// detection is what disarms the alarm after a password unlock.
pub async fn watch(session: SessionCtl, tx: mpsc::Sender<bool>, poll_interval: Duration) {
    match dbus_watch(&session.id(), &tx).await {
        Ok(()) => warn!("logind property stream ended; falling back to polling"),
        Err(e) => warn!(error = %e, "cannot observe logind over D-Bus; falling back to polling"),
    }
    monitor(session, tx, poll_interval).await;
}

/// Subscribe to `LockedHint` changes. Returns `Ok(())` only when the stream ends.
async fn dbus_watch(session_id: &str, tx: &mpsc::Sender<bool>) -> anyhow::Result<()> {
    use futures_util::StreamExt;

    if session_id.is_empty() {
        anyhow::bail!("no session id resolved");
    }

    let conn = zbus::Connection::system().await?;

    let manager = zbus::Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;
    let path: zbus::zvariant::OwnedObjectPath =
        manager.call("GetSession", &(session_id,)).await?;
    debug!(session = %session_id, path = %path.as_str(), "watching logind session over D-Bus");

    let props = zbus::fdo::PropertiesProxy::builder(&conn)
        .destination("org.freedesktop.login1")?
        .path(path)?
        .build()
        .await?;

    let wanted = zbus::names::InterfaceName::try_from("org.freedesktop.login1.Session")?;
    let mut changes = props.receive_properties_changed().await?;

    while let Some(signal) = changes.next().await {
        let args = signal.args()?;
        if args.interface_name != wanted {
            continue;
        }
        if let Some(value) = args.changed_properties.get("LockedHint") {
            match bool::try_from(value.try_clone()?) {
                Ok(locked) => {
                    debug!(locked, "logind reported a lock-state change");
                    if tx.send(locked).await.is_err() {
                        return Ok(()); // the machine shut down
                    }
                }
                Err(e) => warn!(error = %e, "LockedHint was not a boolean"),
            }
        }
    }
    Ok(())
}
```

This API was checked against the `zbus 5.18.0` source already in `Cargo.lock`, so it should compile as written. For reference, `zbus/src/fdo/properties.rs` declares the signal as

```rust
#[zbus(signal)]
pub async fn properties_changed(
    emitter: &SignalEmitter<'_>,
    interface_name: InterfaceName<'_>,
    changed_properties: HashMap<&str, Value<'_>>,
    invalidated_properties: Cow<'_, [&str]>,
) -> zbus::Result<()>;
```

from which the macro generates `receive_properties_changed()` and a `PropertiesChangedArgs` carrying those three fields. `Value::try_clone() -> zvariant::Result<Value>` exists in `zvariant 5.12`. If anything does not compile, adjust the spelling — but **do not** switch to a different mechanism: `PropertiesChanged` on `LockedHint` is the specified one, and logind's `Lock`/`Unlock` signals are not substitutes.

- [ ] **Step 3: Wire it in `main`**

In `crates/alertu-daemon/src/main.rs`, replace the `session::monitor` spawn:

```rust
    // Session lock-state observation: D-Bus when available, polling otherwise.
    tokio::spawn(session::watch(
        session.clone(),
        lock_tx,
        Duration::from_millis(500),
    ));
```

- [ ] **Step 4: Confirm the fallback still works — this is the important test**

The existing `blackbox.rs` test `arming_locks_the_session_and_an_external_unlock_disarms` pins `session_id = "test-session"`, which does not exist, so `GetSession` fails and the daemon must fall back to polling the `loginctl` shim. **That test passing is the fallback's regression test.**

Run: `cargo test -p alertu-daemon --test blackbox`
Expected: PASS — 5 tests, unchanged. If the unlock test now fails or hangs, the fallback is not being reached; fix that before going further.

- [ ] **Step 5: Verify the real path by hand**

With a graphical session available, run the daemon against your own session and confirm it takes the D-Bus path rather than the fallback:

```bash
RUST_LOG=debug target/debug/alertu-daemon --socket /tmp/alertu-dbus.sock --config <a safe config with session_id = "auto">
```

Expected: a `watching logind session over D-Bus` line and **no** "falling back to polling" line. Then lock and unlock the session and confirm `logind reported a lock-state change` appears within milliseconds rather than up to 500 ms.

**The config must still pin** `remote_device`, `watch_devices` and `camera_device` to non-existent paths and leave the webhook empty — only `session_id` may be `"auto"` here, and only because observing the real session is the point. Do not arm the daemon during this check. Kill it and remove the socket afterwards.

If no graphical session is available, say so plainly rather than claiming the check passed.

- [ ] **Step 6: Note the change in the README**

The README's disarm bullet says the password unlock is "detected by polling the session's `LockedHint`". Correct it: detected from logind's `LockedHint` over D-Bus, falling back to polling `loginctl` when the bus is unavailable.

- [ ] **Step 7: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features`
Expected: clean, 92 tests.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(daemon): observe logind lock state over D-Bus, polling as fallback"
```

---

## Verification

| Check | Command | Expected |
| --- | --- | --- |
| Formatting | `cargo fmt --all --check` | silent |
| Lints | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | zero warnings |
| Tests | `cargo test --workspace --all-features` | 81 → 92 |
| Socket boundary | `stat -c %a /tmp/<test socket>` after `blackbox` | `660` |
| Unknown group aborts | `alertu-daemon --socket-group nope` | non-zero exit, no socket created |
| Snapshot boundary | `machine.rs` test | directory `0750` |
| No libdbus | `ldd target/debug/alertu-daemon \| grep -c dbus` | `0` |
| No second reactor | `cargo tree -p alertu-daemon \| grep -c async-io` | `0` |
| Tray icons untouched | `git diff main -- crates/alertu-gui/src/tray.rs` | empty |

## Out of scope

- Moving logind **lock/unlock actions** to D-Bus. They stay on `loginctl`: actions are rare, and keeping them there preserves the `PATH` shim the black-box tests rely on.
- Making `socket_group` a `Config` field. It is a flag precisely so it cannot be changed over the socket it protects.
- Any change to the tray's four state icons.
- Reworking the settings window into an asynchronous client.
