# Lot B — Usability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the sounds AlertU's config already points at, and stop the tray and settings window from dying when the daemon restarts.

**Architecture:** A dependency-free WAV synthesizer in `alertu-ctl` generates the countdown tick and the looping siren; the arm chirp ships as a resource embedded in the binary. `alertu-gui` gains a supervisor loop that reconnects with exponential backoff while the tray icon stays alive, and the shared blocking client gains an opt-in reconnect-and-retry-once for the settings window.

**Tech Stack:** Rust 2021, tokio, ksni, clap. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-07-21-tooling-usability-hardening-design.md` (sections B1–B3)

**Reference implementation:** `docs/superpowers/gensounds.py` is a working Python synthesizer whose output is already committed as `resources/warning.wav` and `resources/siren.wav`, validated by ear on real hardware. Task 1 reproduces it in Rust. Match its constants exactly.

## Global Constraints

- No new system library dependencies: no libasound/rodio, no libdbus, no GTK/Qt. Audio, camera capture and webhooks stay subprocess-based. Pure-Rust crates are acceptable. **This lot adds no new crate dependencies at all.**
- `transitions.rs::decide` stays a pure function with no I/O. This plan does not touch it.
- No behaviour hardcoded for a specific remote model.
- IPC stays newline-delimited JSON with shared types in `alertu-common`.
- Before every commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` (zero warnings), `cargo test --workspace --all-features`.
- 55 tests pass at the start of this lot; none may be lost.
- **The tray's four state-dependent icons (`security-low`, `security-high`, `dialog-warning`, `dialog-error`) must not change.** `icon_name()` and `icon_pixmap()` in `crates/alertu-gui/src/tray.rs` are out of bounds for this lot — the owner asked for them to stay as they are. Task 3 adds a `connected` field and touches the tooltip text and item enablement only.
- Work on a branch `claude/lot-b`. Do not push.

## Decisions already taken (do not relitigate)

- `gen-sounds` synthesizes `warning.wav` and `siren.wav`, and writes `beep.wav` from bytes embedded at compile time from `resources/lock.wav` — the owner's own chirp, chosen over a synthesized sine because it sounds better and is already in the repo.
- The settings window gets a retry that is **opt-in on the client**, never blanket behaviour, so `alertu-ctl` can never silently replay a `toggle`.

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/alertu-ctl/src/sounds.rs` (create) | WAV encoding and synthesis, pure and unit-tested |
| `crates/alertu-ctl/src/main.rs` (modify) | `gen-sounds` subcommand |
| `README.md` (modify) | Document sound installation |
| `crates/alertu-gui/src/main.rs` (modify) | Supervisor loop, session function, backoff |
| `crates/alertu-gui/src/tray.rs` (modify) | `connected` field, tooltip, item enablement |
| `crates/alertu-common/src/ipc_client.rs` (modify) | Opt-in reconnect-and-retry-once |
| `crates/alertu-settings/src/main.rs` (modify) | Enable the retry |

---

### Task 1: The WAV synthesizer

Pure functions with no I/O, so every property the spec calls a requirement is asserted directly.

**Files:**
- Create: `crates/alertu-ctl/src/sounds.rs`
- Modify: `crates/alertu-ctl/src/main.rs` (add `mod sounds;`)

**Interfaces:**
- Consumes: nothing.
- Produces: `sounds::SAMPLE_RATE: u32`, `sounds::encode_wav(&[f32]) -> Vec<u8>`, `sounds::warning_tick() -> Vec<f32>`, `sounds::siren() -> Vec<f32>`.

- [ ] **Step 1: Write the failing tests**

Create `crates/alertu-ctl/src/sounds.rs` containing only this test module for now:

```rust
//! Synthesis of AlertU's default sounds as 16-bit mono PCM WAV.
//!
//! No dependencies: the RIFF header is written by hand and the waveforms are
//! computed directly. Ported from `docs/superpowers/gensounds.py`, whose output
//! was validated on real hardware.

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a little-endian u32 at `off`.
    fn u32_at(bytes: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
    }

    /// Read a little-endian u16 at `off`.
    fn u16_at(bytes: &[u8], off: usize) -> u16 {
        u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap())
    }

    /// Decode the samples back out of an encoded WAV.
    fn samples_of(wav: &[u8]) -> Vec<i16> {
        wav[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    #[test]
    fn header_is_a_valid_mono_16bit_riff() {
        let wav = encode_wav(&[0.0; 100]);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");

        assert_eq!(u32_at(&wav, 16), 16, "PCM fmt chunk is 16 bytes");
        assert_eq!(u16_at(&wav, 20), 1, "format 1 = uncompressed PCM");
        assert_eq!(u16_at(&wav, 22), 1, "mono");
        assert_eq!(u32_at(&wav, 24), SAMPLE_RATE);
        assert_eq!(u32_at(&wav, 28), SAMPLE_RATE * 2, "byte rate");
        assert_eq!(u16_at(&wav, 32), 2, "block align");
        assert_eq!(u16_at(&wav, 34), 16, "bits per sample");
    }

    #[test]
    fn chunk_sizes_match_the_payload() {
        let wav = encode_wav(&[0.0; 100]);
        assert_eq!(wav.len(), 44 + 200);
        assert_eq!(u32_at(&wav, 40), 200, "data chunk size");
        assert_eq!(u32_at(&wav, 4), 36 + 200, "RIFF chunk size");
    }

    #[test]
    fn samples_are_scaled_and_clamped() {
        let wav = encode_wav(&[0.0, 1.0, -1.0, 2.0, -2.0]);
        assert_eq!(samples_of(&wav), vec![0, 32767, -32767, 32767, -32767]);
    }

    #[test]
    fn the_tick_is_short_quiet_and_starts_and_ends_silent() {
        let s = warning_tick();
        // f64, matching the implementation: 44100_f32 * 0.05_f32 can land just
        // under 2205.0 and truncate to 2204.
        assert_eq!(s.len(), (SAMPLE_RATE as f64 * 0.05) as usize);
        assert_eq!(s[0], 0.0);
        assert_eq!(*s.last().unwrap(), 0.0);
        let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.15..=0.20).contains(&peak), "peak was {peak}");
    }

    #[test]
    fn the_siren_is_two_seconds_and_starts_and_ends_silent() {
        let s = siren();
        assert_eq!(s.len(), SAMPLE_RATE as usize * 2);
        assert_eq!(s[0], 0.0);
        assert_eq!(*s.last().unwrap(), 0.0);
        let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.70..=0.80).contains(&peak), "peak was {peak}");
    }

    /// The reason the siren loops without clicking: over one file the phase
    /// advances by a whole number of cycles, so restarting it continues the
    /// waveform. `siren_loop` respawns the player every iteration, so this is a
    /// requirement, not polish.
    #[test]
    fn the_sirens_phase_closes_on_a_whole_number_of_cycles() {
        let advance = SIREN_F_MID * SIREN_SECS;
        assert!(
            (advance - advance.round()).abs() < 1e-9,
            "f_mid * duration must be integral, got {advance}"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p alertu-ctl --lib sounds` — if that reports no target, add `mod sounds;` to `crates/alertu-ctl/src/main.rs` next to `mod render;` first, then run `cargo test -p alertu-ctl sounds`.
Expected: FAIL — `cannot find function 'encode_wav' in this scope`, and the same for `warning_tick`, `siren`, `SAMPLE_RATE`, `SIREN_F_MID`, `SIREN_SECS`.

- [ ] **Step 3: Implement the encoder and the synthesis**

Insert above the test module in `crates/alertu-ctl/src/sounds.rs`:

```rust
/// Sample rate of every generated file.
pub const SAMPLE_RATE: u32 = 44_100;

/// Siren length, in seconds.
pub const SIREN_SECS: f64 = 2.0;
/// Centre frequency of the siren sweep, in hertz.
pub const SIREN_F_MID: f64 = 1000.0;
/// Peak deviation either side of the centre frequency, in hertz.
pub const SIREN_F_DEV: f64 = 400.0;

/// Encode mono samples in `[-1.0, 1.0]` as a 16-bit PCM WAV file.
///
/// Values outside the range are clamped rather than allowed to wrap, which
/// would turn a loud peak into a burst of noise.
pub fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format: uncompressed PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // channels: mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Fade both ends to silence over `ms` milliseconds, in place.
///
/// Every generated file gets this: a waveform that starts or ends on a non-zero
/// sample produces an audible click when a player opens or closes it.
fn fade_ends(samples: &mut [f32], ms: f64) {
    let n = (SAMPLE_RATE as f64 * ms / 1000.0) as usize;
    let n = n.min(samples.len() / 2);
    if n == 0 {
        return;
    }
    let last = samples.len() - 1;
    for i in 0..n {
        let gain = i as f32 / n as f32;
        samples[i] *= gain;
        samples[last - i] *= gain;
    }
}

/// The discreet countdown tick played once a second while `Triggered`.
pub fn warning_tick() -> Vec<f32> {
    const SECS: f64 = 0.05;
    const FREQ: f64 = 1200.0;
    const AMP: f32 = 0.18;

    let n = (SAMPLE_RATE as f64 * SECS) as usize;
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f64 / SAMPLE_RATE as f64;
            AMP * (2.0 * std::f64::consts::PI * FREQ * t).sin() as f32
        })
        .collect();
    fade_ends(&mut out, 6.0);
    out
}

/// The looping alarm siren: a 600↔1400 Hz sweep that restarts seamlessly.
///
/// Instantaneous frequency is `f_mid + f_dev·sin(2πt/T)`, completing exactly one
/// modulation cycle over the file. Integrating gives the phase below; because
/// `f_mid · T` is a whole number, the phase at the end lands a whole number of
/// cycles from the start, so `siren_loop` respawning the player continues the
/// waveform rather than stepping.
pub fn siren() -> Vec<f32> {
    const AMP: f32 = 0.75;

    let n = (SAMPLE_RATE as f64 * SIREN_SECS) as usize;
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f64 / SAMPLE_RATE as f64;
            let phase = 2.0 * std::f64::consts::PI * SIREN_F_MID * t
                - (SIREN_F_DEV * SIREN_SECS)
                    * (2.0 * std::f64::consts::PI * t / SIREN_SECS).cos();
            AMP * phase.sin() as f32
        })
        .collect();
    fade_ends(&mut out, 8.0);
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p alertu-ctl sounds`
Expected: PASS — 7 tests.

- [ ] **Step 5: Confirm the Rust output matches the validated Python reference**

`resources/siren.wav` and `resources/warning.wav` are the Python reference's output, already validated by ear on real hardware. The Rust port should agree on every audible property; byte-identity is not expected, because the Rust fade multiplies in `f32` where Python uses `f64`.

Add this test to the module, keep it, and run it:

```rust
    /// The committed reference files came from `docs/superpowers/gensounds.py`.
    /// The Rust port must agree with them on every property that matters; exact
    /// bytes may differ because the fade multiplies in f32 here, f64 there.
    #[test]
    fn matches_the_committed_reference_files() {
        for (generated, reference) in [
            (encode_wav(&warning_tick()), "../../resources/warning.wav"),
            (encode_wav(&siren()), "../../resources/siren.wav"),
        ] {
            let expected = std::fs::read(reference)
                .unwrap_or_else(|e| panic!("reading {reference}: {e}"));
            assert_eq!(
                generated.len(),
                expected.len(),
                "{reference}: length differs"
            );
            assert_eq!(
                &generated[0..44],
                &expected[0..44],
                "{reference}: header differs"
            );
        }
    }
```

Run: `cargo test -p alertu-ctl sounds`
Expected: PASS. The path is relative to the crate root (`crates/alertu-ctl/`), so `../../resources/`. If the lengths differ, the constants were mistyped — compare against `docs/superpowers/gensounds.py` before changing anything else.

- [ ] **Step 6: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --all-features`
Expected: clean, 62 tests.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: add a dependency-free WAV synthesizer for the default sounds"
```

---

### Task 2: The `gen-sounds` subcommand

**Files:**
- Modify: `crates/alertu-ctl/src/main.rs`, `README.md`

**Interfaces:**
- Consumes: `sounds::{encode_wav, warning_tick, siren}` (Task 1).
- Produces: `alertu-ctl gen-sounds --dir <path> [--force]`.

- [ ] **Step 1: Add the subcommand to the clap definition**

In `crates/alertu-ctl/src/main.rs`, add to `enum Command`:

```rust
    /// Write the default sound files (beep, warning tick, siren) into a directory.
    GenSounds {
        /// Destination directory, e.g. /usr/share/sounds/alertu.
        #[arg(long)]
        dir: PathBuf,
        /// Overwrite files that already exist.
        #[arg(long)]
        force: bool,
    },
```

- [ ] **Step 2: Write the failing tests**

Add to the `tests` module in `crates/alertu-ctl/src/main.rs`:

```rust
    #[test]
    fn gen_sounds_parses_dir_and_force() {
        let cli = Cli::try_parse_from(["alertu-ctl", "gen-sounds", "--dir", "/tmp/s"]).unwrap();
        assert_eq!(
            cli.command,
            Command::GenSounds {
                dir: PathBuf::from("/tmp/s"),
                force: false
            }
        );
        let cli =
            Cli::try_parse_from(["alertu-ctl", "gen-sounds", "--dir", "/tmp/s", "--force"]).unwrap();
        assert!(matches!(cli.command, Command::GenSounds { force: true, .. }));
    }

    #[test]
    fn gen_sounds_requires_a_dir() {
        let err = Cli::try_parse_from(["alertu-ctl", "gen-sounds"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn gen_sounds_writes_three_playable_files() {
        let dir = tempfile::tempdir().unwrap();
        write_sounds(dir.path(), false).unwrap();
        for name in ["beep.wav", "warning.wav", "siren.wav"] {
            let bytes = std::fs::read(dir.path().join(name)).unwrap();
            assert_eq!(&bytes[0..4], b"RIFF", "{name} is not a RIFF file");
            assert_eq!(&bytes[8..12], b"WAVE", "{name} is not a WAVE file");
        }
    }

    #[test]
    fn gen_sounds_refuses_to_clobber_without_force() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("siren.wav"), b"precious").unwrap();

        let err = write_sounds(dir.path(), false).unwrap_err();
        assert!(
            err.to_string().contains("siren.wav"),
            "error should name the file it refused to overwrite, got: {err}"
        );
        assert_eq!(std::fs::read(dir.path().join("siren.wav")).unwrap(), b"precious");

        write_sounds(dir.path(), true).unwrap();
        assert_ne!(std::fs::read(dir.path().join("siren.wav")).unwrap(), b"precious");
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p alertu-ctl gen_sounds`
Expected: FAIL — `cannot find function 'write_sounds' in this scope`.

- [ ] **Step 4: Implement `write_sounds` and dispatch**

Add to `crates/alertu-ctl/src/main.rs`:

```rust
/// The arm/disarm chirp, embedded so `gen-sounds` needs no data files at
/// runtime. Chosen over a synthesized sine because it simply sounds better.
const BEEP_WAV: &[u8] = include_bytes!("../../../resources/lock.wav");

/// Write the three default sounds into `dir`.
///
/// Refuses to overwrite unless `force`, because the natural destination is a
/// system directory a user may have customised.
fn write_sounds(dir: &Path, force: bool) -> Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating {}", dir.display()))?;

    let files: [(&str, Vec<u8>); 3] = [
        ("beep.wav", BEEP_WAV.to_vec()),
        ("warning.wav", sounds::encode_wav(&sounds::warning_tick())),
        ("siren.wav", sounds::encode_wav(&sounds::siren())),
    ];

    for (name, bytes) in &files {
        let path = dir.join(name);
        if path.exists() && !force {
            anyhow::bail!(
                "{} already exists; pass --force to overwrite",
                path.display()
            );
        }
        std::fs::write(&path, bytes)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}
```

Add `mod sounds;` beside `mod render;`, and `use std::path::Path;` to the imports.

`gen-sounds` never touches the socket, so it must be handled before `Client::connect`, exactly like `set-config`'s local validation. In `run`, add as the first branch:

```rust
    if let Command::GenSounds { dir, force } = &cli.command {
        write_sounds(dir, *force)?;
        println!("{}", render::render(&Outcome::Ack, cli.json)?);
        return Ok(());
    }
```

Then add the arm that keeps the main match exhaustive, next to the other early-returned commands, following whatever shape `run` currently uses for `set-config` and `status --watch`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p alertu-ctl`
Expected: PASS — the four new tests plus the existing ones.

- [ ] **Step 6: Generate the sounds for real and listen to them**

Run:

```bash
cargo run -p alertu-ctl -- gen-sounds --dir /tmp/alertu-sounds
paplay /tmp/alertu-sounds/warning.wav
paplay /tmp/alertu-sounds/siren.wav
```

Expected: three files written; the tick is short and quiet, the siren sweeps up and down. Confirm the siren does not click when played twice back to back:

```bash
paplay /tmp/alertu-sounds/siren.wav; paplay /tmp/alertu-sounds/siren.wav
```

- [ ] **Step 7: Document it in the README**

In `README.md`, replace the sentence that currently reads "Put three WAV files where the config points (`beep`, `warning`, `siren`), and" with:

```markdown
Generate the three sound files the config points at, and
```

and insert this command into the install block, before the icon installation:

```sh
sudo alertu-ctl gen-sounds --dir /usr/share/sounds/alertu
```

Also add a line to the `alertu-ctl` command-line section:

```sh
alertu-ctl gen-sounds --dir /usr/share/sounds/alertu   # write the default sounds
```

- [ ] **Step 8: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --all-features`
Expected: clean, 66 tests.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat: add alertu-ctl gen-sounds to write the default sound files"
```

---

### Task 3: Tray reconnection

Today the tray exits the moment the daemon closes the socket: the icon vanishes and the user must restart it by hand. This makes it survive daemon restarts.

**Files:**
- Modify: `crates/alertu-gui/src/main.rs` (substantial restructure), `crates/alertu-gui/src/tray.rs` (additive only)

**Interfaces:**
- Consumes: `AlertuTray`, `Request`, `Response`.
- Produces: nothing consumed by later tasks.

**Hard constraint:** do not modify `icon_name()` or `icon_pixmap()`. The four state icons stay exactly as they are.

- [ ] **Step 1: Add the `connected` flag to the tray**

In `crates/alertu-gui/src/tray.rs`, add the field to the struct after `socket`:

```rust
    /// Whether the daemon connection is currently up. Actions that would send a
    /// request are disabled while it is down, since those requests are dropped.
    pub connected: bool,
```

and initialise it in `new`:

```rust
            connected: false,
```

- [ ] **Step 2: Reflect it in the tooltip and the menu**

In `crates/alertu-gui/src/tray.rs`, replace the body of `tool_tip`:

```rust
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "AlertU".into(),
            description: if self.connected {
                format!("State: {}", self.state)
            } else {
                "Daemon offline — reconnecting…".to_string()
            },
            icon_name: self.icon_name(),
            icon_pixmap: Vec::new(),
        }
    }
```

In `menu`, change the header item's label:

```rust
            StandardItem {
                label: if self.connected {
                    format!("AlertU — {}", self.state)
                } else {
                    "AlertU — daemon offline".to_string()
                },
                enabled: false,
                ..Default::default()
            }
```

Add `enabled: self.connected,` to the Arm/Disarm item and to the "Refresh devices" item. Leave "Open settings…" and "Quit AlertU tray" always enabled — they do not need the daemon.

The three submenus (remote, watch, settings) still send requests that would be dropped while offline. Leaving them enabled is a deliberate scope choice: gating them means threading `connected` through three constructors for no behavioural gain, since a dropped request is already logged. Do not change them.

- [ ] **Step 3: Restructure the supervisor loop**

In `crates/alertu-gui/src/main.rs`, replace everything from `#[tokio::main]` to the end of `main` with:

```rust
/// First retry delay after losing the daemon.
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
/// Ceiling for the exponential backoff.
const MAX_BACKOFF: Duration = Duration::from_secs(10);

/// What one connection attempt achieved, so the supervisor knows whether to
/// reset its backoff.
struct SessionOutcome {
    /// True once any response arrived. Connecting alone is not enough: a socket
    /// that accepts and immediately drops would otherwise never back off.
    got_response: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let socket = socket_path();
    let (req_tx, mut req_rx) = mpsc::unbounded_channel::<Request>();

    // The tray is spawned once and outlives every connection, so the icon never
    // disappears while the daemon restarts.
    let handle = AlertuTray::new(req_tx.clone(), socket.clone())
        .spawn()
        .await
        .context("spawning system tray (need a StatusNotifierItem host / D-Bus session)")?;
    info!("tray registered");

    let mut backoff = INITIAL_BACKOFF;
    loop {
        match run_session(&socket, &mut req_rx, &handle).await {
            Ok(outcome) => {
                if outcome.got_response {
                    backoff = INITIAL_BACKOFF;
                }
            }
            Err(e) => warn!(error = %e, "could not reach the daemon"),
        }
        handle.update(|t| t.connected = false).await;
        warn!(retry_in = ?backoff, "disconnected from daemon");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Hold one connection to the daemon until it drops.
async fn run_session(
    socket: &Path,
    req_rx: &mut mpsc::UnboundedReceiver<Request>,
    handle: &ksni::Handle<AlertuTray>,
) -> Result<SessionOutcome> {
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting to daemon socket {}", socket.display()))?;
    info!(socket = %socket.display(), "connected to daemon");

    // Discard anything the menu queued while we were offline. Replaying a
    // ten-second-old `Arm` against a live alarm would be dangerous, and the
    // unbounded channel would otherwise deliver it now.
    let mut discarded = 0usize;
    while req_rx.try_recv().is_ok() {
        discarded += 1;
    }
    if discarded > 0 {
        warn!(discarded, "dropped requests queued while disconnected");
    }

    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();
    let mut outcome = SessionOutcome {
        got_response: false,
    };

    handle.update(|t| t.connected = true).await;

    // Resynchronise: state pushes, the config, and the device list.
    for req in [Request::Subscribe, Request::GetConfig, Request::ListDevices] {
        write_request(&mut write_half, &req).await?;
    }

    loop {
        tokio::select! {
            maybe = req_rx.recv() => {
                let Some(req) = maybe else {
                    // Every sender is gone; the tray is shutting down.
                    return Ok(outcome);
                };
                if let Err(e) = write_request(&mut write_half, &req).await {
                    warn!(error = %e, "dropping request; the connection is gone");
                    return Ok(outcome);
                }
            }
            line = lines.next_line() => match line {
                Ok(Some(l)) if l.trim().is_empty() => {}
                Ok(Some(l)) => {
                    outcome.got_response = true;
                    match serde_json::from_str::<Response>(&l) {
                        Ok(resp) => apply_response(handle, resp).await,
                        Err(e) => warn!(error = %e, line = %l, "unparseable response"),
                    }
                }
                Ok(None) => {
                    info!("daemon closed the connection");
                    return Ok(outcome);
                }
                Err(e) => {
                    warn!(error = %e, "read error from daemon");
                    return Ok(outcome);
                }
            },
        }
    }
}

/// Serialize one request onto the socket.
async fn write_request<W>(writer: &mut W, req: &Request) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut buf = serde_json::to_vec(req).context("serializing request")?;
    buf.push(b'\n');
    writer.write_all(&buf).await.context("writing request")?;
    writer.flush().await.context("flushing request")?;
    Ok(())
}
```

Keep `socket_path()` and `apply_response()` exactly as they are. The import block becomes:

```rust
use alertu_common::protocol::{Request, Response, DEFAULT_SOCKET_PATH};
use anyhow::{Context, Result};
use ksni::TrayMethods;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tracing::{info, warn};
use tray::AlertuTray;
```

Note `error` is no longer imported: the old code used it for the fatal "daemon closed the connection" path, which is now a routine `info!` because reconnection follows.

- [ ] **Step 4: Build and check for warnings**

Run: `cargo clippy -p alertu-gui --all-targets -- -D warnings`
Expected: zero warnings. Fix any unused import the restructure left behind.

- [ ] **Step 5: Verify reconnection against a real daemon**

This behaviour cannot be unit-tested without a StatusNotifierItem host, so verify it by hand and record the output in your report.

```bash
cargo build
# terminal 1 — a daemon on a short socket path, with a config that touches nothing real
target/debug/alertu-daemon --socket /tmp/alertu-recon.sock --config /tmp/alertu-recon.toml
# terminal 2
RUST_LOG=info target/debug/alertu-gui --socket /tmp/alertu-recon.sock
```

Write `/tmp/alertu-recon.toml` with `remote_device = "/nonexistent/remote"`, `watch_devices = ["/nonexistent/watch"]`, `camera_device = "/nonexistent/video"`, `session_id = "no-such-session"` and the three sound paths pointing at non-existent files, so nothing on the host is touched.

Then: kill the daemon, confirm the tray logs `disconnected from daemon` and stays in the tray with a "Daemon offline" tooltip; restart the daemon, confirm the tray logs `connected to daemon` and resynchronises. Confirm the retry delay grows while the daemon is down and returns to 250 ms after a successful reconnection.

If a StatusNotifierItem host is unavailable in your environment, say so plainly in the report rather than claiming the check passed.

- [ ] **Step 6: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --all-features`
Expected: clean, 66 tests (this task adds none).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(gui): keep the tray alive and reconnect with backoff"
```

---

### Task 4: Settings-window reconnection

**Files:**
- Modify: `crates/alertu-common/src/ipc_client.rs`, `crates/alertu-settings/src/main.rs`

**Interfaces:**
- Consumes: `Client`.
- Produces: `Client::with_reconnect(self) -> Self`.

- [ ] **Step 1: Write the failing tests**

The existing `fake_daemon` helper serves exactly one connection. Add a multi-session variant and two tests to the `tests` module in `crates/alertu-common/src/ipc_client.rs`:

```rust
    /// A fake daemon that serves several connections in turn. Each entry is one
    /// session's replies; an empty slice means "accept, then hang up without
    /// answering", which is what a daemon restart looks like to a client.
    fn fake_daemon_sessions(
        dir: &std::path::Path,
        sessions: &'static [&'static [&'static str]],
    ) -> PathBuf {
        let path = dir.join("multi.sock");
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            for replies in sessions {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                if replies.is_empty() {
                    continue; // drop the connection unanswered
                }
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                for reply in *replies {
                    let _ = writeln!(stream, "{reply}");
                }
                let _ = stream.flush();
            }
        });
        path
    }

    #[test]
    fn without_reconnect_a_dropped_connection_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon_sessions(dir.path(), &[&[], &[r#"{"event":"ok"}"#]]);
        let mut client = Client::connect(&path).unwrap();
        assert!(
            client.set_config(Config::default()).is_err(),
            "a client without reconnect must surface the dropped connection"
        );
    }

    #[test]
    fn with_reconnect_a_dropped_connection_is_retried_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon_sessions(dir.path(), &[&[], &[r#"{"event":"ok"}"#]]);
        let mut client = Client::connect(&path).unwrap().with_reconnect();
        client
            .set_config(Config::default())
            .expect("the retry should have reached the second session");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p alertu-common --features ipc-client reconnect`
Expected: FAIL — `no method named 'with_reconnect'`.

- [ ] **Step 3: Implement the opt-in retry**

In `crates/alertu-common/src/ipc_client.rs`, extend the struct:

```rust
pub struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    /// Kept so the connection can be re-established on demand.
    socket: PathBuf,
    reconnect: bool,
}
```

Add `use std::path::PathBuf;` to the imports, and set the new fields in `connect`:

```rust
        Ok(Client {
            writer,
            reader,
            socket: socket.to_path_buf(),
            reconnect: false,
        })
```

Add the opt-in and the retry:

```rust
    /// Reconnect and retry once when a request fails on I/O.
    ///
    /// Opt-in, and deliberately not the default: a dropped connection is no
    /// evidence about whether the daemon acted on the request, so replaying
    /// `Arm`/`Disarm`/`Toggle` could re-arm an alarm the user just disarmed.
    /// Enable it only for a caller whose requests are idempotent — the settings
    /// window, which sends nothing but `GetConfig`, `ListDevices` and
    /// `SetConfig`.
    pub fn with_reconnect(mut self) -> Self {
        self.reconnect = true;
        self
    }

    /// Re-establish the connection in place, keeping the same `Client`.
    fn reopen(&mut self) -> Result<()> {
        let fresh = Client::connect(&self.socket)?;
        self.writer = fresh.writer;
        self.reader = fresh.reader;
        Ok(())
    }
```

Rename the existing `round_trip` body to `attempt_round_trip` and add the retry wrapper:

```rust
    /// Send one request and read exactly one reply, retrying once through a
    /// fresh connection when [`Client::with_reconnect`] is enabled.
    fn round_trip(&mut self, req: &Request) -> Result<Response> {
        match self.attempt_round_trip(req) {
            Ok(resp) => Ok(resp),
            Err(e) if self.reconnect => {
                self.reopen().context(format!("retrying after: {e}"))?;
                self.attempt_round_trip(req)
            }
            Err(e) => Err(e),
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p alertu-common --features ipc-client`
Expected: PASS — the two new tests plus the existing client tests.

- [ ] **Step 5: Enable it in the settings window**

In `crates/alertu-settings/src/main.rs`, in `reconnect`, change:

```rust
        match Client::connect(&self.socket) {
```

to:

```rust
        match Client::connect(&self.socket).map(Client::with_reconnect) {
```

- [ ] **Step 6: Verify `alertu-ctl` did NOT gain the retry**

Run: `grep -rn "with_reconnect" crates/alertu-ctl/`
Expected: no output. A CLI that silently replays `toggle` would be a bug.

- [ ] **Step 7: Verify the workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --all-features`
Expected: clean, 68 tests.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: reconnect the settings window transparently on I/O failure"
```

---

## Verification

| Check | Command | Expected |
| --- | --- | --- |
| Formatting | `cargo fmt --all --check` | silent |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | zero warnings |
| Tests | `cargo test --workspace --all-features` | 55 → 68 |
| Sounds | `alertu-ctl gen-sounds --dir /tmp/s && paplay /tmp/s/siren.wav` | sweeps, no click on repeat |
| Clobber guard | `alertu-ctl gen-sounds --dir /tmp/s` a second time | fails naming the existing file |
| No CLI retry | `grep -rn with_reconnect crates/alertu-ctl/` | no output |
| Tray icons untouched | `git diff main -- crates/alertu-gui/src/tray.rs \| grep -c icon_name` | only the tooltip's existing call |

## Out of scope for this plan

- Socket permission hardening and `--socket-group` (lot C).
- logind over D-Bus (lot C).
- Snapshot file permissions — found during hardware testing, belongs with lot C's privilege-boundary work.
- Any change to the tray's four state icons.
- Gating the tray's remote/watch/settings submenus on `connected`.
