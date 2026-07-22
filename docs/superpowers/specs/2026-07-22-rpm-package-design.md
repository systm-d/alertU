# RPM Package Design

**Date:** 2026-07-22
**Status:** approved

## Goal

Ship AlertU as a single `alertu` RPM for Fedora, mirroring the Debian package
released in v0.1.0. The release workflow builds both formats on tag and
attaches both to the GitHub release.

## Why this is not a fresh design

The Debian package already settled every question that is about AlertU rather
than about a packaging format: which files ship, where they go, how the service
account is created, what the daemon may write at runtime, and what the user
must be told after installing. This spec inherits those decisions verbatim and
documents only what Fedora genuinely changes.

The one exception is a lesson learned from the Debian work, applied in mirror.
Building the `.deb` on Fedora produced an empty `Depends` field, because
`dpkg-shlibdeps` had no package database to query, and the failure was silent.
The inverse holds: an RPM built on Ubuntu would carry wrong or missing
`Requires`. **The RPM is therefore built inside a `fedora:43` container.** This
is not a preference.

## Distribution

Attached to GitHub releases, alongside the `.deb`. Users run:

```
sudo dnf install ./alertu-0.1.0-1.x86_64.rpm
```

COPR was considered and declined. It is the idiomatic Fedora channel and gives
users automatic updates, but it requires a COPR account, an API token in
repository secrets, and a source RPM — which `cargo-generate-rpm` does not
produce. Revisit if AlertU acquires Fedora users who ask for it; converting the
metadata to a `.spec` at that point is mechanical.

## Tooling

`cargo-generate-rpm` 0.21, mirroring `cargo-deb`. The metadata block lives in
`crates/alertu-daemon/Cargo.toml` immediately after the existing
`[package.metadata.deb]` block, so the two file lists sit side by side and
diverge visibly.

`cargo-generate-rpm` provides `pre_install_script`, `post_install_script`,
`pre_uninstall_script`, `post_uninstall_script` and the `*_trans_script`
variants, plus `--auto-req` for dependency detection. It does **not** expand RPM
macros: there is no `%systemd_post`, and no equivalent of the `#DEBHELPER#`
splice that cargo-deb performs. Every scriptlet is raw shell, written by hand.

## What Fedora changes

### Scriptlet arguments do not mean what they mean on Debian

This is the highest-risk difference and it fails silently.

In a Debian maintainer script, `$1` is a verb (`configure`, `remove`,
`abort-upgrade`). In an RPM scriptlet, `$1` is **the number of versions of the
package that will remain installed after the current operation**:

| Scriptlet | Fresh install | Upgrade | Final removal |
| --- | --- | --- | --- |
| `%post` | `1` | `2` | — |
| `%preun` | — | `1` | `0` |
| `%postun` | — | `1` | `0` |

Transcribing the Debian logic unchanged would produce a package that disables
and stops the service on **every upgrade**, because the Debian `prerm` stops
the unit unconditionally and relies on `postinst` to start it again. The RPM
scriptlets must gate on these numbers:

- `%post`: run `systemd-sysusers`, `systemctl daemon-reload`, and — only when
  `$1 -eq 1` — `systemctl preset alertu-daemon`. Refresh the icon cache.
  Create `/etc/alertu` owned by `alertu`.
- `%preun`: only when `$1 -eq 0`, `systemctl --no-reload disable --now
  alertu-daemon`.
- `%postun`: `systemctl daemon-reload`. When `$1 -ge 1` (upgrade), restart the
  daemon so the new binary takes effect.

`systemctl preset` rather than `enable`: it honours the administrator's preset
policy, which is the Fedora convention and lets a site decline auto-enabling.

### Dependencies are declared by file path

RPM resolves file-path dependencies through the `Provides` of every package in
the repository, which decouples the requirement from any particular package
name. This matters concretely for the webcam.

The daemon resolves its tools at runtime by binary name — `fswebcam` if
present, otherwise `ffmpeg` (`snapshot.rs`), and `paplay`, then `pw-play`, then
`aplay` (`sound.rs`). On Fedora, `/usr/bin/ffmpeg` is provided by `ffmpeg-free`
from the Fedora repositories, while the package literally named `ffmpeg` lives
in RPM Fusion, a third-party repository. **Requiring `ffmpeg` by name would
force users to enable RPM Fusion.** Requiring the path does not.

- `Requires`: `/usr/bin/loginctl` (systemd; the daemon cannot lock a session
  without it), plus whatever `--auto-req` resolves for the shared libraries.
- `Recommends` (weak dependencies, honoured by dnf, silently ignored when
  unresolvable, removable by the user): `fswebcam`, then `/usr/bin/ffmpeg`,
  then `/usr/bin/paplay` for sound. `fswebcam` is named first for the same
  reason as on Debian — it is a fraction of ffmpeg's install footprint and does
  the same job.

  One item needs checking during implementation: `fswebcam` is in the Debian
  archive, but its presence in the Fedora repositories was not verified while
  writing this spec, because `dnf repoquery` could not read its configuration
  in the sandbox. Confirm it inside the build container. If it is absent,
  the recommendation is harmless — an unresolvable weak dependency is ignored
  — but the ordering rationale no longer applies and `/usr/bin/ffmpeg` becomes
  the effective default, which the README must then say.

  `/usr/bin/paplay` rather than a sound package name: `paplay` is the daemon's
  first choice and is provided by `pulseaudio-utils` on Fedora, while
  `pipewire-utils` provides `pw-play` and `alsa-utils` provides `aplay`. A
  desktop install already has at least one; the path form avoids asserting
  which audio stack the user runs.

The GUI's X11, Wayland and GL libraries are `dlopen`ed by winit and glutin, so
`--auto-req` cannot see them, exactly as `$auto` could not on Debian. They are
declared as `Recommends` under their Fedora names: `mesa-libGL`,
`libxkbcommon`.

### Keeping snapshots on removal is free

The Debian package needed an explicit `postrm` branch to spare
`/var/lib/alertu/snapshots` from purge. Files created at runtime by systemd's
`StateDirectory` are not in the RPM manifest, so `dnf remove` leaves them
untouched with no code at all. Same guarantee, less machinery.

### SELinux requires nothing

Verified on Fedora 43 Workstation with SELinux enforcing: the running daemon's
context is `system_u:system_r:unconfined_service_t:s0`. A systemd service with
no dedicated policy module inherits this permissive domain, and systemd's
`RuntimeDirectory` and `StateDirectory` label `/run/alertu` and
`/var/lib/alertu` correctly. No policy module, no `restorecon` in a scriptlet.

If AlertU is ever submitted to Fedora proper, a confined domain becomes a
requirement — out of scope here.

### There is no conffile concept, but nothing ships under /etc anyway

RPM marks configuration with `%config(noreplace)` rather than tracking
checksums the way dpkg does. The Debian constraint stands unchanged and for the
same reason: the daemon **rewrites** its own config whenever the tray, the
settings window or `alertu-ctl set-config` saves. A packaged config file would
be fought over by the package manager and the application. The package ships
`config.example.toml` under `/usr/share/doc/alertu/` and creates an empty,
`alertu`-owned `/etc/alertu` directory in `%post`.

## Inherited invariants

Carried from the Debian package without modification. Each was established by a
review finding, not by preference:

1. **No file under `/etc` in the payload.**
2. **The package never adds anyone to the `alertu` group.** Socket membership
   is a privilege grant — it confers full command of the alarm — so the package
   prints the `usermod` instruction and stops.
3. **The tray's user unit ships but is never enabled by the package.** A
   system-level scriptlet has no business touching a user's systemd instance.
   The post-install message and the README both carry
   `systemctl --user enable --now alertu-gui`; without it the user gets a
   working daemon and no tray, which was Important finding #1 on the Debian
   branch.
4. **Sounds install 0644**, readable by the `alertu` service account.
5. **First-use guidance prints on fresh install only**, never on upgrade.

## CI changes

`.github/workflows/release.yml` gains a second build path. The existing Debian
job is untouched. The RPM job runs on `ubuntu-latest` with
`container: fedora:43`, installs the Rust toolchain, `rpm-build` (which provides
`/usr/lib/rpm/find-requires`, without which `--auto-req` silently degrades) and
the eframe build dependencies, then builds, packages, installs, verifies and
attaches.

The verification mirrors what the Debian job learned to assert:

- the produced `Requires` is non-empty and contains a `libc.so.6` symbol
  requirement — the assertion that catches a silent `--auto-req` failure, which
  is the exact class of bug that shipped an empty `Depends` from Fedora;
- `dnf install` succeeds and the package reaches `installed` state;
- `systemctl is-active alertu-daemon` — not `is-enabled`, which passes for a
  unit that enabled and then crashed.

Both artefacts attach to the same release. The `gh release create || gh release
upload` fallback already in place handles either tagging flow.

## Testing

A `fedora:43` container with systemd as PID 1, the protocol that validated the
Debian package and closed the gap that was documented as unverifiable:

1. The daemon starts as `User=alertu` under `ProtectSystem=strict`.
2. `/run/alertu/alertu.sock` exists as `srw-rw---- alertu:alertu`.
3. `alertu-ctl set-config` persists to `/etc/alertu/config.toml`.
4. The post-install message appears on first install and is suppressed on
   reinstall.
5. **Install → upgrade → remove**, asserting the daemon keeps running across
   the upgrade. This is where the `$1` semantics fail, and nothing else catches
   it.
6. Snapshots under `/var/lib/alertu/snapshots` survive `dnf remove`.

`rpmlint` is run and its output reported. Findings are triaged, not
automatically fixed: the Debian equivalent left several accepted warnings.

## Compatibility floor

The RPM is built on `fedora:43`. Unlike the Debian package — whose `libc6 (>=
2.39)` floor comes from two weak `pidfd` symbols in Rust's std and rules out
Debian 12 — RPM auto-requires record the specific glibc **symbol versions**
actually referenced, which is a weaker and more accurate constraint.

The floor will therefore be **measured on the produced package and documented
from that measurement**, not asserted in advance. The README states whatever
the artefact actually requires.

## Migration on an existing manual install

Identical in shape to the Debian guidance, and it applies to the maintainer's
own machine, which currently runs a manual install from `/usr/local/bin` with
`/etc/systemd/system/alertu-daemon.service`.

Without cleanup the old install wins silently: `/usr/local/bin` precedes
`/usr/bin` in `PATH`, and a unit in `/etc/systemd/system` overrides one in
`/usr/lib/systemd/system`. The package appears to install correctly while the
old binaries keep running. The commands to run **before** installing go in the
README next to the Debian block.

## Out of scope

- COPR publication.
- A confined SELinux policy module.
- Any architecture other than `x86_64`.
- Submission to Fedora proper.
- Changing any Rust source. This is packaging only.
