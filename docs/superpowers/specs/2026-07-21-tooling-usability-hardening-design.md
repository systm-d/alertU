# AlertU — Tooling, Usability & Hardening

**Date:** 2026-07-21
**Status:** Approved, ready for planning
**Scope:** The seven follow-up tasks queued after the initial scaffold, grouped
into three lots delivered in sequence.

## Goal

The scaffold works end to end but is only drivable through a GUI, is not
covered by any integration test, ships without the sounds it references, dies
when the daemon restarts, and leaves its control socket world-writable. This
design closes all five gaps.

## Constraints (inherited, non-negotiable)

- No new system library dependencies: no libasound/rodio, no libdbus, no
  GTK/Qt. Audio, camera capture and webhooks stay subprocess-based. Pure-Rust
  crates (including FFI-to-glibc crates such as `libc`) are acceptable.
- `transitions.rs::decide` stays a pure function with no I/O, exhaustively
  covered by unit tests. Every side effect goes through `Effect` and the
  interpreter in `machine.rs`.
- No behaviour hardcoded for a specific remote model.
- IPC stays newline-delimited JSON with shared types in `alertu-common`.
- `cargo fmt`, `cargo clippy --workspace` (zero warnings) and `cargo test` must
  pass before every commit.

## Delivery order

Dependencies dictate the sequence. Lot A first because the CLI is the test
client for its own integration tests, and CI should only be switched on once
those tests are stable.

| Lot | Item | Depends on |
| --- | --- | --- |
| A | `alertu_common::ipc_client` | — |
| A | `alertu-ctl` | ipc_client |
| A | daemon lib/bin split | — |
| A | integration tests | ctl, lib split |
| A | GitHub Actions CI | integration tests |
| B | sound generator | ctl |
| B | tray reconnection | ipc_client |
| B | settings-window reconnection | ipc_client |
| C | socket hardening | — |
| C | logind over D-Bus | — |

---

## Lot A — Tooling & verification

### A1. `alertu_common::ipc_client`

`alertu-settings/src/client.rs` moves verbatim to
`alertu-common/src/ipc_client.rs`, behind a cargo feature `ipc-client` that is
**not** enabled by default. It needs no new dependencies: `serde_json` is
already a dependency of `alertu-common` and everything else is `std`
(`UnixStream`, `BufReader`).

The API gains the verbs the settings window never needed:

```rust
impl Client {
    pub fn connect(socket: &Path) -> Result<Client>;
    pub fn get_state(&mut self) -> Result<GuardState>;
    pub fn get_config(&mut self) -> Result<Config>;
    pub fn set_config(&mut self, cfg: Config) -> Result<()>;
    pub fn list_devices(&mut self) -> Result<Vec<InputDeviceInfo>>;
    pub fn arm(&mut self) -> Result<()>;
    pub fn disarm(&mut self) -> Result<()>;
    pub fn toggle(&mut self) -> Result<()>;
    /// Register for pushes; returns the state snapshot sent in reply.
    pub fn subscribe(&mut self) -> Result<GuardState>;
    /// Block until the next pushed Response (only valid after `subscribe`).
    pub fn next_push(&mut self) -> Result<Response>;
}
```

`alertu-settings` keeps only a `use alertu_common::ipc_client::Client;` and
enables the feature in its manifest. `client.rs` is deleted.

### A2. `alertu-ctl`

New binary crate `crates/alertu-ctl`, depending on `alertu-common` with the
`ipc-client` feature plus `clap` (derive). Introducing `clap` breaks the
hand-rolled argument parsing used by the daemon and the tray; that is a
deliberate, approved exception — the CLI is the user-facing surface where good
`--help` output and subcommand parsing earn their keep. The daemon and tray are
not converted.

```
alertu-ctl [--socket <path>] [--json] <command>

  arm                            Force-arm (locks the session)
  disarm                         Force-disarm (unlocks the session)
  toggle                         Toggle, same as a remote button press
  status [--watch]               Print current state; --watch streams changes
  get-config                     Print the daemon's effective config as TOML
  set-config <file.toml | ->     Replace the daemon config (`-` reads stdin)
  list-devices                   List input devices seen by the daemon
  gen-sounds --dir <path> [--force]   Write the default WAV files (see B1)
```

Behaviour details:

- `--socket` defaults to `DEFAULT_SOCKET_PATH`.
- `set-config` parses and calls `Config::validate()` **locally** before the
  round trip, so typos produce an immediate, readable error instead of a
  generic daemon rejection.
- `status --watch` sends `Subscribe` then prints one line per pushed
  transition, until interrupted.
- `--json` prints the raw `Response` as JSON instead of the human rendering.
  This is what makes the CLI scriptable and usable as a test client.
- `gen-sounds` is the one subcommand that does not talk to the socket.

Exit codes: `0` success, `1` daemon or connection error, `2` usage error
(`clap` already uses `2` for parse failures, so this is consistent).

Tests: `alertu-ctl` is deliberately a thin shell over `ipc_client`, so it is
covered by unit tests rather than by spawning a daemon:

- argument parsing, via `clap`'s `try_parse_from` on representative argument
  vectors (including the failure cases that must exit `2`);
- output rendering, by keeping the human and `--json` renderers as pure
  `Response -> String` functions that are asserted directly;
- `gen-sounds`, covered by the WAV unit tests in B1.

The socket round trip itself is covered once, in `blackbox.rs` (A4), through
the same `ipc_client` code the CLI uses.

Rejected: end-to-end tests spawning the daemon from `alertu-ctl`'s test crate.
`CARGO_BIN_EXE_alertu-daemon` is only defined for integration tests of the
crate that declares that binary, so the CLI tests would have to guess the
daemon's path relative to their own executable and force it into the build
graph. That fragility buys nothing that the split above does not already cover.

### A3. Daemon lib/bin split

`alertu-daemon` gains `src/lib.rs` declaring the existing modules as `pub mod`.
`src/main.rs` shrinks to argument parsing plus the wiring already in `main`,
importing from the library. Behaviour is unchanged; this exists solely to let
integration tests construct a `Machine` in-process.

Items that must become `pub` for tests: `machine::{Machine, Channels, Control}`,
`input::InputSignal`, `session::SessionCtl`, `sound::SoundPlayer`.

### A4. Integration tests

Two files under `crates/alertu-daemon/tests/`.

**`blackbox.rs`** — drives the real binary.

Setup: a temporary directory holding `config.toml`, a `bin/` subdirectory
containing a `loginctl` shim, and the socket. The daemon is spawned from
`env!("CARGO_BIN_EXE_alertu-daemon")` with `--config` and `--socket` pointing
into that directory and `PATH` prefixed with the shim directory.

The shim is a shell script that appends its `argv` to a log file and answers
`show-session <id> -p LockedHint --value` by printing the contents of a state
file the test controls. It also answers `show-seat`/`list-sessions` with a
fixed session id so `resolve_session_id` succeeds.

**Guard rail:** the test config must pin `remote_device` and `watch_devices` to
non-existent paths. Leaving them at the `"auto"` default would make the daemon
open the test machine's real keyboard and mouse. Sound paths are likewise
pointed at non-existent files, which the daemon already tolerates with a warning.

Scenario:

1. Wait for the socket to appear, connect with `ipc_client`.
2. `arm()` → poll until the shim log contains `lock-session`; assert
   `get_state() == Armed`.
3. Flip the `LockedHint` state file to `no` → the session monitor observes the
   change → poll until `get_state() == Idle`.

All waits are bounded polls with a timeout (a few seconds) that fail the test
with a diagnostic rather than hanging.

**`machine.rs`** — in-process state machine wiring.

Builds a `Machine` directly with `grace_period_secs = 0` and
`alarm_delay_secs = 1`, pushes `InputSignal::Activity` onto `sig_tx`, and
asserts the progression `Armed → Triggered → Alarm` by observing the
`watch::Receiver<GuardState>`. This covers the interpreter and the timer paths
that `transitions.rs` unit tests cannot reach, without needing evdev, uinput or
any privilege.

Rejected alternatives: uinput virtual devices (needs `/dev/uinput` plus `input`
group membership, unusable in CI) and a feature-gated `Request::InjectInput`
(adds a control-plane backdoor and makes the tested binary differ from the
shipped one).

### A5. CI

`.github/workflows/ci.yml`, `ubuntu-latest`, single job:

1. `actions/checkout`
2. Rust toolchain with `rustfmt` + `clippy`
3. Cargo cache
4. `apt-get install -y` the packages `eframe` needs to build
   (`alertu-settings`): `libxkbcommon-dev`, `libxkbcommon-x11-dev`,
   `libwayland-dev`, `libgl1-mesa-dev`, `libx11-dev`, `libxcursor-dev`,
   `libxrandr-dev`, `libxi-dev`. The exact list is confirmed by the first CI
   run and trimmed to what actually fails without it. Approved over excluding
   the crate from CI: roughly 30 seconds of CI time buys full workspace
   coverage.
5. `cargo fmt --all --check`
6. `cargo clippy --workspace --all-targets -- -D warnings`
7. `cargo test --workspace`

---

## Lot B — Usability

### B1. Default sounds

Module `sounds.rs` in `alertu-ctl`, writing 16-bit mono PCM WAV at 44.1 kHz
with a hand-written RIFF header. No dependencies.

| File | Content | Duration |
| --- | --- | --- |
| `beep.wav` | 2000 Hz sine, fast attack and decay | ~120 ms |
| `warning.wav` | 1200 Hz tick at ~15 % amplitude | ~40 ms |
| `siren.wav` | 600↔1400 Hz sweep, full amplitude | ~2 s |

Two properties are requirements, not polish, because `siren_loop` restarts the
player on every iteration:

- The siren must start and end at zero amplitude and at the same phase,
  otherwise every loop boundary produces an audible click.
- All three files carry a few milliseconds of fade at both ends.

Unit tests assert the RIFF/WAVE/`fmt `/`data` chunk structure, the `data`
length matching the expected frame count, the resulting duration, and that the
first and last samples are near zero (the anti-click and clean-loop guarantee).

`gen-sounds` refuses to overwrite an existing file unless `--force` is passed,
because its natural target is a system directory. The default config already
points at `/usr/share/sounds/alertu`; the README documents
`alertu-ctl gen-sounds --dir /usr/share/sounds/alertu` as an install step. No
new packaging machinery.

### B2. Tray reconnection

The current structure blocks reconnection: `req_rx` is owned by a dedicated
writer task, so it cannot be recovered once the connection drops. Restructure
`alertu-gui/src/main.rs`:

- Spawn the tray **once**, outside the loop, so the icon never disappears.
- A supervisor loop runs `connect → session → backoff → retry`. Backoff is
  exponential: 250 ms, doubling, capped at 10 s. It resets to 250 ms as soon as
  a connection receives its first response from the daemon (the reply to
  `Subscribe`) — connecting alone is not enough, since a socket that accepts
  and immediately drops would otherwise never back off.
- The session task runs a single `select!` over `req_rx` and incoming lines,
  and **returns `req_rx`** to the supervisor when it exits.
- Requests emitted while disconnected are **dropped with a warning, not
  queued**. Replaying a ten-second-old `Arm` against an alarm would be
  dangerous.
- `AlertuTray` gains `connected: bool`, driving a "daemon offline" tooltip and
  greyed-out action items.
- Every successful reconnection replays `Subscribe`, `GetConfig` and
  `ListDevices` to resynchronise.

### B3. Settings-window reconnection

The settings window is synchronous and only performs discrete round trips;
restructuring it the way the tray is restructured would be disproportionate.
Instead, `ipc_client::Client` reconnects lazily on the first I/O error and
replays the request **once**.

This is safe here specifically because the settings window only ever sends
`GetConfig`, `ListDevices` and `SetConfig`, all idempotent. It never sends
`Arm`, `Disarm` or `Toggle`. The retry is therefore implemented as an opt-in
method on the client (e.g. a `reconnecting` flag set by the settings window),
not as blanket behaviour for every consumer — `alertu-ctl` must not silently
replay a `toggle`.

If the retry also fails, the window shows an error banner instead of exiting.

---

## Lot C — Hardening

### C1. Socket permissions

- `ipc::serve` takes the desired group as a parameter.
- Socket mode goes from `0o666` to `0o660`.
- `alertu-daemon` gains `--socket-group <name>`.
- The systemd unit changes `RuntimeDirectoryMode` from `0755` to `0750`.
- Name-to-gid resolution uses `libc::getgrnam_r` (pure FFI to glibc; no
  additional system library).
- **Hard failure**: an unknown group, or a failing `chown`, aborts startup. The
  daemon must never fall back to a socket more permissive than requested.

With no flag, everything works out of the box: the socket and the runtime
directory both inherit the service's primary group (`alertu`), so
`usermod -aG alertu <user>` is the whole setup.

When `--socket-group <other>` **is** passed, the daemon also applies that group
to the socket's parent directory. Without this, `/run/alertu` stays
`alertu:alertu 0750`, the target group cannot traverse it, and the flag is
silently inoperative. The directory is recreated by systemd on every start, so
the change has no persistent effect.

The README threat-model note becomes explicit: socket access is equivalent to
full control — disarming, reading the config (including the webhook URL), and
`SetConfig`, which steers the paths handed to the helper programs. Group
membership is a privilege boundary, not a convenience.

### C2. logind over D-Bus

Replaces the current 500 ms `loginctl` poll — two subprocess spawns per second,
indefinitely — with real-time observation over zbus (pure Rust; explicitly not
libdbus).

A precision that shapes the implementation: on
`org.freedesktop.login1.Session`, the `Lock` and `Unlock` signals are
**requests addressed to the screen locker**, not notifications of a state
change. The observable state change is `PropertiesChanged` on `LockedHint`.
That is the exact real-time equivalent of what is polled today, and that is
what the daemon subscribes to, after resolving the object path via
`Manager.GetSession(id)`.

**Scope split (approved):** D-Bus is used for **observation only**.
`lock`/`unlock` **actions** keep shelling out to `loginctl`. Three reasons:
observation is what costs, while actions are rare; keeping `loginctl` for
actions preserves the `PATH` shim used by the A4 black-box tests, which would
otherwise need a simulated D-Bus service; and it keeps the change surface small
on sensitive code.

Polling remains as a fallback: if the bus connection or the property-change
subscription fails, the daemon logs a warning and falls back to the existing
`session::monitor` loop. The public interface of `SessionCtl` does not change,
so `machine.rs` is untouched.

`zbus` is a new dependency of `alertu-daemon` (it is already an indirect
dependency of `alertu-gui` through `ksni`).

### C3. Snapshot permissions

*Added after the first hardware run, which surfaced this.*

Alarm snapshots are written by `fswebcam`/`ffmpeg` as subprocesses, so the files
inherit the child's umask — observed as `0640` on a stock Fedora. Under the
systemd unit the daemon runs as `alertu`, so the photos land `alertu:alertu`
`0640` and **the desktop user cannot read them**. The failure is silent: nothing
reports it, and the user simply finds no usable evidence after an alarm.

This is the same privilege boundary as C1, so it gets the same answer rather
than a second one: the snapshot directory becomes `0750` owned by the socket
group, and each captured file is `chmod`ed to `0640` after a successful capture.
Members of the group — the desktop user, by C1's setup — can read the photos;
nobody else can.

**Not `0644`.** These are webcam photographs of whoever is at the machine,
including the owner. World-readable by default would be a privacy regression
introduced in the name of convenience. The group boundary already exists; reuse
it.

The `chmod` happens after the capture rather than by setting the child's umask,
because the capture is a subprocess: adjusting its umask means `pre_exec` and
`unsafe`, while a post-capture `set_permissions` on a path the daemon already
knows is deterministic and needs neither.

### C4. A testable tray session loop

*Added after lot B, whose review established the gap.*

`alertu-gui`'s reconnection logic has no automated coverage beyond the extracted
backoff function, because `ksni::TrayMethods::spawn()` always registers with a
`StatusNotifierWatcher` and offers no headless mode. Two behaviours that carry
real safety content are therefore unverified: that `Subscribe`, `GetConfig` and
`ListDevices` are replayed **in order on every reconnection**, and that requests
queued while disconnected are **dropped rather than replayed** — the guard
against a ten-second-old `Arm` firing against a live alarm.

Decouple `run_session` from `ksni::Handle` behind a two-method trait:

```rust
/// What a session does to the UI. Implemented by `ksni::Handle<AlertuTray>` in
/// production and by a recording fake in tests.
trait TrayView {
    async fn set_connected(&self, connected: bool);
    async fn apply(&self, response: Response);
}
```

`run_session` then takes `&impl TrayView` and can be driven against a plain
`tokio::net::UnixListener` with no D-Bus at all. The tray itself is untouched;
this is a seam, not a redesign.

---

## Testing summary

| Area | Coverage |
| --- | --- |
| `ipc_client` | Exercised by every `alertu-ctl` integration test |
| `alertu-ctl` | Unit tests: argument parsing, output rendering, exit codes |
| Daemon wiring | `blackbox.rs`: arm → lock, external unlock → disarm |
| Interpreter & timers | `machine.rs`: Armed → Triggered → Alarm |
| Sound generator | Unit tests: header, length, duration, edge samples |
| Tray reconnection | Backoff unit-tested; the session loop covered via `TrayView` (C4) |
| Socket hardening | Unit test on group resolution; `blackbox.rs` asserts the mode |
| D-Bus session | Manual verification; fallback path exercised by `blackbox.rs` |
| Snapshot permissions | Unit test on the chmod helper; `machine.rs` asserts the directory's mode after reaching `Alarm` (no camera involved — capture fails on a non-existent device *after* creating the directory) |
| Session loop | `TrayView` fake: resync order, stale-request drop, clean exit |

Existing coverage (23 tests, notably the exhaustive `transitions.rs` table)
stays green throughout.

## Out of scope

- Converting the daemon and tray to `clap`.
- Moving logind **actions** to D-Bus.
- Any packaging beyond the systemd unit changes and README documentation.
- Reworking the settings window into an asynchronous client.
