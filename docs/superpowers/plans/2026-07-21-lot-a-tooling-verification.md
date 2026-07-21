# Lot A — Tooling & Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give AlertU a scriptable CLI, a reusable IPC client, real integration tests for the daemon, and CI.

**Architecture:** The synchronous socket client moves out of `alertu-settings` into `alertu-common` behind an off-by-default cargo feature, so every consumer shares one implementation. A new `alertu-ctl` binary wraps it. `alertu-daemon` gains a library target so integration tests can build a `Machine` in-process, alongside a black-box test that spawns the real binary with a fake `loginctl` on `PATH`.

**Tech Stack:** Rust 2021, tokio, serde/serde_json, clap (derive, new), tempfile (new, dev-only), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-07-21-tooling-usability-hardening-design.md` (sections A1–A5)

## Global Constraints

- No new system library dependencies: no libasound/rodio, no libdbus, no GTK/Qt. Audio, camera capture and webhooks stay subprocess-based. Pure-Rust crates (including FFI-to-glibc crates such as `libc`) are acceptable.
- `transitions.rs::decide` stays a pure function with no I/O. This plan does not modify it.
- No behaviour hardcoded for a specific remote model.
- IPC stays newline-delimited JSON with shared types in `alertu-common`.
- Before every commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` (zero warnings), `cargo test --workspace`.
- Work happens on branch `claude/lots-abc`. Do not push to `main`.
- The existing 23 tests must stay green throughout.

## Safety Constraints For Tests

These are not style preferences — violating them makes `cargo test` damage the developer's machine.

- **Never let a test config keep the `"auto"` defaults for `remote_device` / `watch_devices`.** `"auto"` makes the daemon open the real keyboard and mouse of the machine running the tests. Always pin them to non-existent paths.
- **Always pin `camera_device` to a non-existent path and `snapshot_dir` into the temp directory.** `snapshot::capture` (`crates/alertu-daemon/src/snapshot.rs:11`) does *not* check that the camera device exists; it invokes `fswebcam`/`ffmpeg` directly. A test reaching `Alarm` on a machine with either tool installed would otherwise trigger the real webcam.
- **Always pin `session_id` to a non-existent id such as `"test-session"`.** `SessionCtl::lock()` shells out to `loginctl lock-session <id>`. With a real session id and a real `loginctl` on `PATH`, a test would lock the developer's screen. A bogus id makes the call fail harmlessly.
- Leave `alarm_webhook_url` empty; `webhook::fire` returns immediately on an empty URL.

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/alertu-common/src/ipc_client.rs` (create) | Blocking request/response socket client, feature-gated |
| `crates/alertu-common/src/lib.rs` (modify) | Declare the feature-gated module |
| `crates/alertu-common/Cargo.toml` (modify) | `ipc-client` feature, `tempfile` dev-dependency |
| `crates/alertu-settings/src/client.rs` (delete) | Superseded by the shared client |
| `crates/alertu-settings/src/main.rs` (modify) | Import the shared client |
| `crates/alertu-ctl/src/main.rs` (create) | clap CLI definition, dispatch, exit codes |
| `crates/alertu-ctl/src/render.rs` (create) | `Outcome` + pure rendering functions (unit-tested) |
| `crates/alertu-daemon/src/lib.rs` (create) | Library face exposing the daemon modules |
| `crates/alertu-daemon/src/main.rs` (modify) | Thin binary importing from the library |
| `crates/alertu-daemon/tests/blackbox.rs` (create) | Spawns the real binary; arm→lock, unlock→disarm |
| `crates/alertu-daemon/tests/machine.rs` (create) | In-process Armed→Triggered→Alarm |
| `.github/workflows/ci.yml` (create) | fmt / clippy / test |

---

### Task 1: Move the IPC client into `alertu-common`

A pure refactor: the code moves unchanged, and `alertu-settings` keeps working. No new behaviour, so the gate is "everything still builds and the existing tests pass".

**Files:**
- Create: `crates/alertu-common/src/ipc_client.rs`
- Modify: `crates/alertu-common/src/lib.rs`, `crates/alertu-common/Cargo.toml`
- Modify: `crates/alertu-settings/src/main.rs:9,14`, `crates/alertu-settings/Cargo.toml`
- Delete: `crates/alertu-settings/src/client.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `alertu_common::ipc_client::Client` with `connect(&Path) -> Result<Client>`, `get_state() -> Result<GuardState>`, `get_config() -> Result<Config>`, `set_config(Config) -> Result<()>`, `list_devices() -> Result<Vec<InputDeviceInfo>>`.

- [ ] **Step 1: Move the file unchanged**

```bash
git mv crates/alertu-settings/src/client.rs crates/alertu-common/src/ipc_client.rs
```

- [ ] **Step 2: Add the feature and the dev-dependency**

In `crates/alertu-common/Cargo.toml`, append after the `[dependencies]` block:

```toml
[features]
# The blocking socket client. Off by default so the daemon, which has its own
# async IPC server, does not compile a client it never uses.
ipc-client = []

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Declare the module**

In `crates/alertu-common/src/lib.rs`, after `pub mod state;`:

```rust
#[cfg(feature = "ipc-client")]
pub mod ipc_client;
```

- [ ] **Step 4: Fix the module's own imports**

In `crates/alertu-common/src/ipc_client.rs`, the three `use alertu_common::…` lines become crate-relative:

```rust
use crate::config::Config;
use crate::protocol::{InputDeviceInfo, Request, Response};
use crate::state::GuardState;
```

- [ ] **Step 5: Enable the feature in `alertu-settings`**

In `crates/alertu-settings/Cargo.toml`, replace the `alertu-common` line:

```toml
alertu-common = { path = "../alertu-common", features = ["ipc-client"] }
```

- [ ] **Step 6: Rewire the settings window**

In `crates/alertu-settings/src/main.rs`, delete line 9 (`mod client;`) and replace line 14 (`use client::Client;`) with:

```rust
use alertu_common::ipc_client::Client;
```

- [ ] **Step 7: Verify the whole workspace still builds and tests pass**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: builds clean, 23 tests pass.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor: move the blocking IPC client into alertu-common"
```

---

### Task 2: Add the remote-control verbs to the client

**Files:**
- Modify: `crates/alertu-common/src/ipc_client.rs`

**Interfaces:**
- Consumes: `Client` from Task 1.
- Produces: `Client::arm()`, `Client::disarm()`, `Client::toggle()` (all `-> Result<()>`), `Client::subscribe() -> Result<GuardState>`, `Client::next_push() -> Result<Response>`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/alertu-common/src/ipc_client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread;

    /// Start a fake daemon in a background thread: it accepts one connection,
    /// reads a single request line, then writes every reply in `replies`.
    /// Returns the socket path.
    fn fake_daemon(dir: &std::path::Path, replies: &'static [&'static str]) -> PathBuf {
        let path = dir.join("fake.sock");
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            for reply in replies {
                writeln!(stream, "{reply}").unwrap();
            }
            stream.flush().unwrap();
        });
        path
    }

    #[test]
    fn arm_accepts_an_ok_reply() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon(dir.path(), &[r#"{"event":"ok"}"#]);
        let mut client = Client::connect(&path).unwrap();
        client.arm().unwrap();
    }

    #[test]
    fn an_error_reply_becomes_an_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon(dir.path(), &[r#"{"event":"error","message":"nope"}"#]);
        let mut client = Client::connect(&path).unwrap();
        let err = client.toggle().unwrap_err();
        assert!(err.to_string().contains("nope"), "got: {err}");
    }

    #[test]
    fn an_unexpected_reply_becomes_an_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon(dir.path(), &[r#"{"event":"state","state":"idle"}"#]);
        let mut client = Client::connect(&path).unwrap();
        assert!(client.disarm().is_err());
    }

    #[test]
    fn subscribe_returns_the_snapshot_then_streams_pushes() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon(
            dir.path(),
            &[
                r#"{"event":"state","state":"idle"}"#,
                r#"{"event":"state_changed","state":"armed"}"#,
            ],
        );
        let mut client = Client::connect(&path).unwrap();
        assert_eq!(client.subscribe().unwrap(), GuardState::Idle);
        assert_eq!(
            client.next_push().unwrap(),
            Response::StateChanged {
                state: GuardState::Armed
            }
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p alertu-common --features ipc-client`
Expected: FAIL — `no method named 'arm' found`, plus the same for `toggle`, `disarm`, `subscribe`, `next_push`.

- [ ] **Step 3: Refactor `round_trip` to share the read path**

In `crates/alertu-common/src/ipc_client.rs`, replace the body of `round_trip` and add `read_response`:

```rust
    /// Send one request and read exactly one reply.
    fn round_trip(&mut self, req: &Request) -> Result<Response> {
        let mut line = serde_json::to_string(req).context("serializing request")?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .context("writing request")?;
        self.writer.flush().context("flushing request")?;
        self.read_response()
    }

    /// Read a single newline-delimited response off the socket.
    fn read_response(&mut self) -> Result<Response> {
        let mut buf = String::new();
        let n = self.reader.read_line(&mut buf).context("reading reply")?;
        if n == 0 {
            return Err(anyhow!("daemon closed the connection"));
        }
        serde_json::from_str(buf.trim_end()).context("parsing reply")
    }
```

- [ ] **Step 4: Add the verbs**

Add inside `impl Client`, after `set_config`:

```rust
    /// Force-arm, locking the session.
    pub fn arm(&mut self) -> Result<()> {
        self.expect_ok(&Request::Arm)
    }

    /// Force-disarm, unlocking the session.
    pub fn disarm(&mut self) -> Result<()> {
        self.expect_ok(&Request::Disarm)
    }

    /// Toggle arm/disarm, exactly as a remote button press would.
    pub fn toggle(&mut self) -> Result<()> {
        self.expect_ok(&Request::Toggle)
    }

    /// Register for asynchronous pushes; returns the state snapshot the daemon
    /// sends in reply. Follow with [`Client::next_push`].
    pub fn subscribe(&mut self) -> Result<GuardState> {
        match self.round_trip(&Request::Subscribe)? {
            Response::State { state } => Ok(state),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(anyhow!("unexpected reply to Subscribe: {other:?}")),
        }
    }

    /// Block until the daemon pushes the next response. Only meaningful after
    /// [`Client::subscribe`]; on a non-subscribed connection this blocks forever.
    pub fn next_push(&mut self) -> Result<Response> {
        self.read_response()
    }

    /// Round-trip a request whose only successful answer is a bare `Ok`.
    fn expect_ok(&mut self, req: &Request) -> Result<()> {
        match self.round_trip(req)? {
            Response::Ok => Ok(()),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(anyhow!("unexpected reply to {req:?}: {other:?}")),
        }
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p alertu-common --features ipc-client`
Expected: PASS — 4 new tests plus the existing `alertu-common` tests.

- [ ] **Step 6: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean, all tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: add arm/disarm/toggle/subscribe verbs to the IPC client"
```

---

### Task 3: The `alertu-ctl` CLI

Rendering lives in `render.rs` as pure `Outcome -> String` functions so it is unit-testable without a daemon. `main.rs` only parses, dispatches and maps errors to exit codes.

Note: the `gen-sounds` subcommand described in spec section A2 is implemented in **Lot B** (B1). Do not add it here.

**Files:**
- Create: `crates/alertu-ctl/Cargo.toml`, `crates/alertu-ctl/src/main.rs`, `crates/alertu-ctl/src/render.rs`
- Modify: `Cargo.toml` (workspace members and dependencies)

**Interfaces:**
- Consumes: `alertu_common::ipc_client::Client` (Tasks 1–2).
- Produces: binary `alertu-ctl`. `render::Outcome` with variants `State(GuardState)`, `Config(Box<Config>)`, `Devices(Vec<InputDeviceInfo>)`, `Ack`; `render::render(&Outcome, json: bool) -> anyhow::Result<String>`.

- [ ] **Step 1: Register the crate and the clap dependency**

In the workspace `Cargo.toml`, add to `members`:

```toml
    "crates/alertu-ctl",
```

and to `[workspace.dependencies]`:

```toml
clap = { version = "4", features = ["derive"] }
```

- [ ] **Step 2: Create the manifest**

Create `crates/alertu-ctl/Cargo.toml`:

```toml
[package]
name = "alertu-ctl"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "AlertU command-line control: arm, disarm, status, config and devices over the daemon socket."

[[bin]]
name = "alertu-ctl"
path = "src/main.rs"

[dependencies]
alertu-common = { path = "../alertu-common", features = ["ipc-client"] }
anyhow.workspace = true
clap.workspace = true
serde_json.workspace = true
toml.workspace = true
```

- [ ] **Step 3: Write the failing rendering tests**

Create `crates/alertu-ctl/src/render.rs`:

```rust
//! Turning daemon replies into terminal output.
//!
//! Rendering is kept separate from the socket round trip, and pure, so the
//! human and `--json` forms can be asserted directly in unit tests without a
//! running daemon.

use alertu_common::config::Config;
use alertu_common::protocol::{InputDeviceInfo, Response};
use alertu_common::state::GuardState;
use anyhow::{Context, Result};

/// What a command produced, before rendering.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    State(GuardState),
    Config(Box<Config>),
    Devices(Vec<InputDeviceInfo>),
    /// A command that only needed acknowledgement (arm/disarm/toggle/set-config).
    Ack,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn device(path: &str, name: &str, keyboard: bool, pointer: bool) -> InputDeviceInfo {
        InputDeviceInfo {
            path: PathBuf::from(path),
            name: name.to_string(),
            is_keyboard: keyboard,
            is_pointer: pointer,
        }
    }

    #[test]
    fn human_state_is_the_bare_label() {
        assert_eq!(render(&Outcome::State(GuardState::Armed), false).unwrap(), "Armed");
    }

    #[test]
    fn human_ack_is_ok() {
        assert_eq!(render(&Outcome::Ack, false).unwrap(), "ok");
    }

    #[test]
    fn human_devices_list_path_name_and_tags() {
        let out = render(
            &Outcome::Devices(vec![
                device("/dev/input/event3", "Fake Keyboard", true, false),
                device("/dev/input/event4", "Fake Mouse", false, true),
            ]),
            false,
        )
        .unwrap();
        assert_eq!(
            out,
            "/dev/input/event3  Fake Keyboard [keyboard]\n/dev/input/event4  Fake Mouse [pointer]"
        );
    }

    #[test]
    fn human_devices_says_so_when_empty() {
        assert_eq!(render(&Outcome::Devices(vec![]), false).unwrap(), "no input devices");
    }

    #[test]
    fn human_config_is_toml_that_round_trips() {
        let out = render(&Outcome::Config(Box::new(Config::default())), false).unwrap();
        let back: Config = toml::from_str(&out).unwrap();
        assert_eq!(back, Config::default());
    }

    #[test]
    fn json_state_matches_the_wire_format() {
        let out = render(&Outcome::State(GuardState::Idle), true).unwrap();
        assert_eq!(out, r#"{"event":"state","state":"idle"}"#);
    }

    #[test]
    fn json_ack_matches_the_wire_format() {
        assert_eq!(render(&Outcome::Ack, true).unwrap(), r#"{"event":"ok"}"#);
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p alertu-ctl`
Expected: FAIL — `cannot find function 'render' in this scope`. (`main.rs` does not exist yet, so if cargo complains about a missing binary target, create an empty `src/main.rs` with `fn main() {}` first — Step 6 fills it in.)

- [ ] **Step 5: Implement the rendering**

Insert into `crates/alertu-ctl/src/render.rs`, between `Outcome` and the test module:

```rust
impl Outcome {
    /// The protocol response this outcome came from, so `--json` emits exactly
    /// the daemon's wire format rather than a second, divergent shape.
    fn to_response(&self) -> Response {
        match self {
            Outcome::State(state) => Response::State { state: *state },
            Outcome::Config(cfg) => Response::Config(cfg.clone()),
            Outcome::Devices(devices) => Response::Devices {
                devices: devices.clone(),
            },
            Outcome::Ack => Response::Ok,
        }
    }
}

/// Render an outcome for the terminal, as JSON or in human form.
pub fn render(outcome: &Outcome, json: bool) -> Result<String> {
    if json {
        return serde_json::to_string(&outcome.to_response()).context("serializing JSON output");
    }
    Ok(match outcome {
        Outcome::State(state) => state.to_string(),
        Outcome::Ack => "ok".to_string(),
        Outcome::Config(cfg) => toml::to_string_pretty(&**cfg)
            .context("serializing config as TOML")?
            .trim_end()
            .to_string(),
        Outcome::Devices(devices) => render_devices(devices),
    })
}

fn render_devices(devices: &[InputDeviceInfo]) -> String {
    if devices.is_empty() {
        return "no input devices".to_string();
    }
    devices
        .iter()
        .map(|d| {
            let mut tags = Vec::new();
            if d.is_keyboard {
                tags.push("keyboard");
            }
            if d.is_pointer {
                tags.push("pointer");
            }
            let tags = if tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", tags.join(","))
            };
            format!("{}  {}{}", d.path.display(), d.name, tags)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
```

- [ ] **Step 6: Write the CLI with its parsing tests**

Create `crates/alertu-ctl/src/main.rs`:

```rust
//! AlertU command-line control.
//!
//! A thin, scriptable wrapper over the daemon socket: everything the tray and
//! the settings window can do, plus a `--json` mode that emits the raw
//! protocol responses so shell scripts can consume them.

mod render;

use alertu_common::config::Config;
use alertu_common::ipc_client::Client;
use alertu_common::protocol::{Response, DEFAULT_SOCKET_PATH};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use render::Outcome;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "alertu-ctl",
    version,
    about = "Control the AlertU daemon over its Unix socket"
)]
struct Cli {
    /// Path to the daemon's control socket.
    #[arg(long, short, global = true, default_value = DEFAULT_SOCKET_PATH)]
    socket: PathBuf,

    /// Emit the raw protocol response as JSON instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug, PartialEq)]
enum Command {
    /// Force-arm: lock the session and start watching.
    Arm,
    /// Force-disarm: unlock the session and stop watching.
    Disarm,
    /// Toggle arm/disarm, exactly as a remote button press would.
    Toggle,
    /// Print the current guard state.
    Status {
        /// Keep running, printing one line per state change.
        #[arg(long)]
        watch: bool,
    },
    /// Print the daemon's effective configuration.
    GetConfig,
    /// Replace the daemon's configuration from a TOML file (`-` reads stdin).
    SetConfig {
        /// Path to a TOML config, or `-` for stdin.
        file: PathBuf,
    },
    /// List the input devices the daemon can see.
    ListDevices,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("alertu-ctl: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    let mut client = Client::connect(&cli.socket)?;

    // `status --watch` streams instead of producing a single outcome.
    if let Command::Status { watch: true } = &cli.command {
        let state = client.subscribe()?;
        println!("{}", render::render(&Outcome::State(state), cli.json)?);
        loop {
            match client.next_push()? {
                Response::State { state } | Response::StateChanged { state } => {
                    println!("{}", render::render(&Outcome::State(state), cli.json)?);
                }
                // Device-list pushes also arrive on a subscribed connection;
                // they are not state changes, so `status` ignores them.
                Response::Devices { .. } | Response::Config(_) | Response::Ok => {}
                Response::Error { message } => anyhow::bail!(message),
            }
        }
    }

    let outcome = match &cli.command {
        Command::Arm => {
            client.arm()?;
            Outcome::Ack
        }
        Command::Disarm => {
            client.disarm()?;
            Outcome::Ack
        }
        Command::Toggle => {
            client.toggle()?;
            Outcome::Ack
        }
        Command::Status { watch: false } => Outcome::State(client.get_state()?),
        Command::Status { watch: true } => unreachable!("handled above"),
        Command::GetConfig => Outcome::Config(Box::new(client.get_config()?)),
        Command::SetConfig { file } => {
            client.set_config(read_config(file)?)?;
            Outcome::Ack
        }
        Command::ListDevices => Outcome::Devices(client.list_devices()?),
    };

    println!("{}", render::render(&outcome, cli.json)?);
    Ok(())
}

/// Read and validate a config locally, so a typo produces a precise error here
/// rather than a generic rejection after a round trip.
fn read_config(file: &PathBuf) -> Result<Config> {
    let text = if file.as_os_str() == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading config from stdin")?;
        buf
    } else {
        std::fs::read_to_string(file)
            .with_context(|| format!("reading config {}", file.display()))?
    };
    let cfg: Config = toml::from_str(&text).context("parsing config TOML")?;
    cfg.validate().context("validating config")?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn socket_defaults_to_the_protocol_constant() {
        let cli = Cli::try_parse_from(["alertu-ctl", "status"]).unwrap();
        assert_eq!(cli.socket, PathBuf::from(DEFAULT_SOCKET_PATH));
        assert!(!cli.json);
    }

    #[test]
    fn global_flags_are_accepted_after_the_subcommand() {
        let cli = Cli::try_parse_from(["alertu-ctl", "status", "--json", "-s", "/tmp/a.sock"])
            .unwrap();
        assert!(cli.json);
        assert_eq!(cli.socket, PathBuf::from("/tmp/a.sock"));
    }

    #[test]
    fn status_watch_is_off_unless_asked() {
        let cli = Cli::try_parse_from(["alertu-ctl", "status"]).unwrap();
        assert_eq!(cli.command, Command::Status { watch: false });
        let cli = Cli::try_parse_from(["alertu-ctl", "status", "--watch"]).unwrap();
        assert_eq!(cli.command, Command::Status { watch: true });
    }

    #[test]
    fn set_config_takes_a_path_and_accepts_stdin() {
        let cli = Cli::try_parse_from(["alertu-ctl", "set-config", "/tmp/c.toml"]).unwrap();
        assert_eq!(
            cli.command,
            Command::SetConfig {
                file: PathBuf::from("/tmp/c.toml")
            }
        );
        let cli = Cli::try_parse_from(["alertu-ctl", "set-config", "-"]).unwrap();
        assert_eq!(
            cli.command,
            Command::SetConfig {
                file: PathBuf::from("-")
            }
        );
    }

    #[test]
    fn an_unknown_subcommand_is_a_usage_error() {
        let err = Cli::try_parse_from(["alertu-ctl", "explode"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn a_missing_subcommand_is_a_usage_error() {
        let err = Cli::try_parse_from(["alertu-ctl"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn set_config_without_a_file_is_a_usage_error() {
        let err = Cli::try_parse_from(["alertu-ctl", "set-config"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }
}
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p alertu-ctl`
Expected: PASS — 7 rendering tests and 7 CLI tests.

- [ ] **Step 8: Sanity-check the generated help**

Run: `cargo run -p alertu-ctl -- --help`
Expected: usage listing `arm`, `disarm`, `toggle`, `status`, `get-config`, `set-config`, `list-devices`, plus the global `--socket` and `--json`.

- [ ] **Step 9: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean, all tests pass.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat: add alertu-ctl command-line control"
```

---

### Task 4: Split `alertu-daemon` into a library and a binary

Behaviour must not change. The gate is that the existing daemon tests still pass and the binary still builds.

**Files:**
- Create: `crates/alertu-daemon/src/lib.rs`
- Modify: `crates/alertu-daemon/src/main.rs:6-15,17-27`, `crates/alertu-daemon/Cargo.toml`

**Interfaces:**
- Consumes: nothing.
- Produces: library `alertu_daemon` exposing `machine::{Machine, Channels, Control}`, `input::InputSignal`, `session::{SessionCtl, monitor}`, `sound::SoundPlayer`, `ipc`, `hotplug`, `devices`, `transitions`, `snapshot`, `webhook`.

- [ ] **Step 1: Create the library root**

Create `crates/alertu-daemon/src/lib.rs`:

```rust
//! Library face of the AlertU daemon.
//!
//! The binary in `main.rs` is a thin wiring layer over these modules. They are
//! public so integration tests can drive the state machine in-process, without
//! evdev devices or any privilege.

pub mod devices;
pub mod hotplug;
pub mod input;
pub mod ipc;
pub mod machine;
pub mod session;
pub mod snapshot;
pub mod sound;
pub mod transitions;
pub mod webhook;
```

- [ ] **Step 2: Declare the library target**

In `crates/alertu-daemon/Cargo.toml`, insert before the `[[bin]]` block:

```toml
[lib]
name = "alertu_daemon"
path = "src/lib.rs"
```

- [ ] **Step 3: Make the binary use the library**

In `crates/alertu-daemon/src/main.rs`, delete the ten `mod …;` lines (lines 6–15) and replace the import block so that lines 17–27 read:

```rust
use alertu_common::config::Config;
use alertu_common::protocol::DEFAULT_SOCKET_PATH;
use alertu_common::state::GuardState;
use alertu_daemon::machine::{Channels, Control, Machine};
use alertu_daemon::session::{self, SessionCtl};
use alertu_daemon::sound::SoundPlayer;
use alertu_daemon::{hotplug, ipc};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
```

The rest of `main.rs` is unchanged: `machine::Machine::new(...)`, `session::monitor(...)`, `hotplug::watch(...)` and `ipc::serve(...)` all resolve through these imports.

- [ ] **Step 4: Verify nothing changed behaviourally**

Run: `cargo build --workspace && cargo test --workspace`
Expected: builds clean, the existing daemon unit tests (`transitions`, `machine`, `devices`, `hotplug`) still pass.

- [ ] **Step 5: Verify lints on the newly public API**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: zero warnings. Making modules public can surface lints that only apply to public items; fix any by adding the missing trait impl or `#[allow]` with a comment, never by weakening the lint globally.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: expose alertu-daemon as a library for integration tests"
```

---

### Task 5: Black-box integration test

Spawns the real daemon binary with a fake `loginctl` on `PATH` and drives it through the shared IPC client.

**Files:**
- Create: `crates/alertu-daemon/tests/blackbox.rs`
- Modify: `crates/alertu-daemon/Cargo.toml` (dev-dependencies)

**Interfaces:**
- Consumes: the `alertu-daemon` binary via `env!("CARGO_BIN_EXE_alertu-daemon")`; `alertu_common::ipc_client::Client` (Tasks 1–2).
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Add the dev-dependencies**

In `crates/alertu-daemon/Cargo.toml`, append:

```toml
[dev-dependencies]
alertu-common = { path = "../alertu-common", features = ["ipc-client"] }
tempfile = "3"
```

- [ ] **Step 2: Write the failing test**

Create `crates/alertu-daemon/tests/blackbox.rs`:

```rust
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
    wait_until(Duration::from_secs(10), "the daemon socket to accept", || {
        Client::connect(&harness.socket).is_ok()
    });
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
        client.get_state().map(|s| s == GuardState::Armed).unwrap_or(false)
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
    wait_until(Duration::from_secs(5), "the external unlock to disarm", || {
        client.get_state().map(|s| s == GuardState::Idle).unwrap_or(false)
    });
}

#[test]
fn toggle_arms_then_disarms_and_unlocks() {
    let harness = start();
    let mut client = harness.connect();

    client.toggle().unwrap();
    wait_until(Duration::from_secs(5), "toggle to arm", || {
        client.get_state().map(|s| s == GuardState::Armed).unwrap_or(false)
    });

    client.toggle().unwrap();
    wait_until(Duration::from_secs(5), "toggle to disarm", || {
        client.get_state().map(|s| s == GuardState::Idle).unwrap_or(false)
    });

    let log = harness.shim_log();
    assert!(log.contains("lock-session"), "shim log:\n{log}");
    assert!(log.contains("unlock-session"), "shim log:\n{log}");
}
```

- [ ] **Step 3: Run the test to verify it fails for the right reason**

Run: `cargo test -p alertu-daemon --test blackbox`
Expected: compiles and runs. If it fails, the failure must be a timeout panic with the message from `wait_until`, not a compile error. A compile error here means Task 4 left something private.

- [ ] **Step 4: Confirm it passes**

Run: `cargo test -p alertu-daemon --test blackbox -- --nocapture`
Expected: PASS, 2 tests. Both tests spawn their own daemon, so they are independent and safe to run in parallel.

- [ ] **Step 5: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean, all tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "test: add black-box daemon integration test with a loginctl shim"
```

---

### Task 6: In-process state machine integration test

Covers the interpreter and the timer paths that `transitions.rs` cannot reach, with no binary, no evdev and no privilege.

**Files:**
- Create: `crates/alertu-daemon/tests/machine.rs`

**Interfaces:**
- Consumes: `alertu_daemon::{machine::{Machine, Channels, Control}, input::InputSignal, session::SessionCtl, sound::SoundPlayer}` (Task 4).
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Write the failing test**

Create `crates/alertu-daemon/tests/machine.rs`:

```rust
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
use alertu_daemon::session::SessionCtl;
use alertu_daemon::sound::SoundPlayer;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// A config that touches nothing real. See the plan's safety constraints:
/// `session_id` must not name a real logind session or `loginctl lock-session`
/// would lock the developer's screen, and `camera_device` must not exist or
/// reaching `Alarm` would trigger the webcam.
fn test_config() -> Config {
    Config {
        remote_device: "/nonexistent/remote".to_string(),
        watch_devices: vec!["/nonexistent/watch".to_string()],
        grace_period_secs: 0,
        alarm_delay_secs: 1,
        beep_sound: PathBuf::from("/nonexistent/beep.wav"),
        warning_sound: PathBuf::from("/nonexistent/warning.wav"),
        siren_sound: PathBuf::from("/nonexistent/siren.wav"),
        camera_device: "/nonexistent/video".to_string(),
        session_id: "test-session".to_string(),
        alarm_webhook_url: String::new(),
        ..Config::default()
    }
}

/// Wait until the observed state equals `want`, or fail after `timeout`.
async fn expect_state(
    rx: &mut watch::Receiver<GuardState>,
    want: GuardState,
    timeout: Duration,
) {
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

#[tokio::test]
async fn intrusion_while_armed_escalates_to_the_alarm_then_disarms() {
    let cfg = test_config();
    let dir = tempfile::tempdir().unwrap();

    let (sig_tx, sig_rx) = mpsc::channel(64);
    let (_lock_tx, lock_rx) = mpsc::channel(16);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<Control>(32);
    let (state_tx, mut state_rx) = watch::channel(GuardState::Idle);
    let (devices_tx, _devices_rx) = watch::channel(Vec::<InputDeviceInfo>::new());

    // Our own handle for injecting intrusion; the machine keeps its clone for
    // the evdev readers it spawns (which all fail to open, harmlessly).
    let injector = sig_tx.clone();

    let session = SessionCtl::new(&cfg).await;
    let machine = Machine::new(
        cfg,
        dir.path().join("config.toml"),
        session,
        SoundPlayer::new(),
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

    ctrl_tx.send(Control::Arm).await.unwrap();
    expect_state(&mut state_rx, GuardState::Armed, Duration::from_secs(5)).await;

    injector
        .send(InputSignal::Activity {
            source: "test device".to_string(),
        })
        .await
        .unwrap();
    expect_state(&mut state_rx, GuardState::Triggered, Duration::from_secs(5)).await;

    // `alarm_delay_secs = 1`, so the siren fires about a second later.
    expect_state(&mut state_rx, GuardState::Alarm, Duration::from_secs(5)).await;

    ctrl_tx.send(Control::Disarm).await.unwrap();
    expect_state(&mut state_rx, GuardState::Idle, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn activity_during_the_grace_period_is_ignored() {
    let mut cfg = test_config();
    cfg.grace_period_secs = 60;
    let dir = tempfile::tempdir().unwrap();

    let (sig_tx, sig_rx) = mpsc::channel(64);
    let (_lock_tx, lock_rx) = mpsc::channel(16);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<Control>(32);
    let (state_tx, mut state_rx) = watch::channel(GuardState::Idle);
    let (devices_tx, _devices_rx) = watch::channel(Vec::<InputDeviceInfo>::new());

    let injector = sig_tx.clone();
    let session = SessionCtl::new(&cfg).await;
    let machine = Machine::new(
        cfg,
        dir.path().join("config.toml"),
        session,
        SoundPlayer::new(),
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

    ctrl_tx.send(Control::Arm).await.unwrap();
    expect_state(&mut state_rx, GuardState::Armed, Duration::from_secs(5)).await;

    injector
        .send(InputSignal::Activity {
            source: "test device".to_string(),
        })
        .await
        .unwrap();

    // Give the machine time to (not) react, then assert it stayed put.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(*state_rx.borrow_and_update(), GuardState::Armed);
}
```

- [ ] **Step 2: Run the tests to verify they compile and fail meaningfully**

Run: `cargo test -p alertu-daemon --test machine`
Expected: compiles. A compile error means Task 4 left `Machine`, `Channels`, `Control`, `InputSignal`, `SessionCtl` or `SoundPlayer` private.

- [ ] **Step 3: Confirm they pass**

Run: `cargo test -p alertu-daemon --test machine -- --nocapture`
Expected: PASS, 2 tests, taking roughly 2 seconds total.

- [ ] **Step 4: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean, all tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test: cover the state machine interpreter and its timers in-process"
```

---

### Task 7: Continuous integration

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the whole workspace as built by Tasks 1–6.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Write the workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always

jobs:
  check:
    name: fmt / clippy / test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Cache cargo
        uses: Swatinem/rust-cache@v2

      # alertu-settings builds eframe, which needs the X11/Wayland/GL headers.
      - name: Install eframe build dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
            libgl1-mesa-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev

      - name: Format
        run: cargo fmt --all --check

      - name: Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: Test
        run: cargo test --workspace
```

- [ ] **Step 2: Reproduce the CI steps locally**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all three succeed. This is exactly what CI runs.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add fmt/clippy/test workflow"
```

- [ ] **Step 4: Confirm the first CI run and trim the package list**

After pushing the branch, watch the "Install eframe build dependencies" and "Test" steps. If the build succeeds, remove packages from the list one at a time to find the minimal set; if it fails on a missing header, add the package the error names. Commit the corrected list.

```bash
gh run watch
```

---

## Verification

After Task 7, the following must all hold:

| Check | Command | Expected |
| --- | --- | --- |
| Formatting | `cargo fmt --all --check` | silent |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | zero warnings |
| Tests | `cargo test --workspace` | 23 pre-existing + 4 client + 15 render/CLI + 2 black-box + 2 machine = 46 |
| CLI help | `cargo run -p alertu-ctl -- --help` | lists all seven subcommands |
| No feature leak | `cargo build -p alertu-daemon` | builds without the `ipc-client` feature |

## Out of scope for this plan

- `gen-sounds` and the WAV generator (Lot B, task B1).
- Tray and settings-window reconnection (Lot B, tasks B2/B3).
- Socket permission hardening and `--socket-group` (Lot C, task C1).
- logind over D-Bus (Lot C, task C2).
- Converting `alertu-daemon` or `alertu-gui` to clap.
