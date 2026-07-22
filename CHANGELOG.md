# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-22

Adds a Fedora package. Nothing about the alarm itself changed.

### Added

- **Fedora RPM**, mirroring the Debian package file for file, built by
  `cargo-generate-rpm` and attached to each release beside the `.deb`. It is
  built inside a `fedora:43` container rather than on the CI runner's Ubuntu:
  `--auto-req` resolves against the local RPM database, so building elsewhere
  produces a package that claims to need nothing — the failure the Debian
  package shipped in reverse, silently, when built on Fedora.
- The release workflow now installs each package in a container with systemd as
  PID 1 and asserts the daemon is actually running, rather than that its unit is
  merely enabled. For the RPM it also upgrades over the installed package and
  asserts the daemon survives: RPM scriptlet arguments count the versions that
  will remain installed, where a Debian maintainer script receives a verb, and
  transcribing the Debian control flow would stop the daemon on every upgrade
  and never restart it, with no error printed anywhere.

### Changed

- The staging script that rewrites unit paths and generates the sounds moved
  from `packaging/deb/stage.sh` to `packaging/stage.sh`, writing to
  `target/pkg-staging`. Neither job it does is Debian-specific.

### Notes

- Unlike the `.deb`, the RPM does not start the daemon. `%post` runs
  `systemctl preset`, honouring the administrator's preset policy, and Fedora's
  default policy leaves third-party units disabled — so `sudo systemctl enable
  --now alertu-daemon` is part of the documented install.
- The RPM requires glibc 2.39, which means Fedora 40 or newer. The floor is
  measured from the built package's symbol requirements and gated in CI, so it
  cannot drift away from what the README promises.

## [0.1.0] - 2026-07-22

First working version. A cheap USB or Bluetooth HID remote acts as a key fob:
click to arm, the session locks, and anyone who touches the machine trips a
countdown, then a siren, a webcam still and an optional webhook. Verified
end to end on real hardware.

### Added

- **Daemon** — generic evdev reading (no model hardcoded), a pure state
  machine interpreted separately from its side effects, logind session control,
  audio and webcam capture as subprocesses, `/dev/input` hotplug, and a
  newline-delimited JSON control socket.
- **Tray** (`alertu-gui`) — StatusNotifierItem via pure-Rust `ksni`, reflecting
  the guard state, with device selection and delays in its menu. Survives a
  daemon restart: it reconnects with exponential backoff instead of exiting, and
  drops requests queued while offline rather than replaying a stale `arm`.
- **Settings window** (`alertu-settings`) — egui/eframe, editing the whole
  configuration and pushing it live. Reconnects transparently on I/O failure.
- **Command line** (`alertu-ctl`) — `arm`, `disarm`, `toggle`, `status`
  (with `--watch`), `get-config`, `set-config`, `list-devices`, `gen-sounds`.
  A `--json` mode emits the daemon's raw protocol responses. Exit codes: `0`
  success, `1` daemon or connection error, `2` usage error.
- **Sounds** — `alertu-ctl gen-sounds` writes the three default files with modes
  the daemon's service account can actually read.
- Packaging: systemd units, a `sysusers.d` service account, a freedesktop icon
  and desktop entry.
- CI running formatting, clippy with `-D warnings`, and the full test suite.

### Changed

Recorded here rather than under a later version: these landed before v0.1.0 was
tagged and shipped in it, but were left sitting under `[Unreleased]`.

- Moved to Rust edition 2024, with a declared MSRV of 1.88 checked in CI.
- Pinned the toolchain (`rust-toolchain.toml`) and formatting (`rustfmt.toml`),
  and moved lints from scattered source attributes into `[workspace.lints]`.
- The alarm siren is now a recording embedded in `alertu-ctl` rather than a
  synthesized sweep. Only the countdown tick is still generated — it has to be a
  few tens of milliseconds, since a player is spawned once a second.
- Sound files moved to `crates/alertu-ctl/assets/`, so `cargo package` can
  include them. Every crate now carries the metadata needed to publish.

### Security

- The control socket is `0660` with an optional `--socket-group`, not the
  world-connectable `0666` it started as. Connecting grants full command of the
  alarm, so group membership is a privilege boundary.
- Alarm snapshots get the same boundary: directory `0750`, files `0640`.
  Deliberately not world-readable — they are photographs of whoever is at the
  machine.
- Directories the daemon does not own are left alone with a warning rather than
  re-permissioned, so pointing `--socket` or `snapshot_dir` at a shared
  directory cannot wreck it.

[Unreleased]: https://github.com/systm-d/alertU/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/systm-d/alertU/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/systm-d/alertU/releases/tag/v0.1.0
