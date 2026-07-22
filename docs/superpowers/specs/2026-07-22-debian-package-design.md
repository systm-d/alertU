# AlertU — Debian package

**Date:** 2026-07-22
**Status:** Approved, ready for planning
**Scope:** A `.deb` built in CI and attached to GitHub releases, for Debian and
Ubuntu users. Not for the maintainer's own machine, which runs Fedora.

## Goal

AlertU currently installs by running a shell script that copies binaries,
creates a service account, generates sounds and enables a unit. That is fine for
its author and unacceptable for anyone else: nothing is tracked, nothing
uninstalls, nothing upgrades.

A `.deb` makes the install reviewable (`dpkg -c`), reversible (`apt remove`) and
upgradable, and it declares what AlertU needs rather than assuming it.

## Constraints and decisions already taken

- **One package**, `alertu`, containing all four binaries. Not split into
  `alertu` + `alertu-gui`, though the GUI drags in X11/Wayland/GL libraries the
  daemon does not need.
- **No configuration file is shipped.** See "Why no conffile" below.
- **Built in CI**, attached to GitHub releases on a tag. Not a local-only target.
- MIT licence, unchanged. crates.io publishing is deferred and out of scope.

## Why no conffile

Debian expects `/etc/alertu/config.toml` to be a *conffile*: `dpkg` tracks its
checksum so an upgrade never silently overwrites local edits.

That collides with a feature we deliberately built. `SetConfig` **persists** —
the tray's device picker, the settings window and `alertu-ctl set-config` all
write that file. A conffile rewritten by a running service makes `dpkg` ask
"keep your version or the maintainer's?" on every single upgrade, forever.

So the package ships **no config at all**. This works because
`Config::load` already returns defaults when the file is missing
(`crates/alertu-common/src/config.rs`), and `Config::save` creates its parent
directory. The post-install script creates `/etc/alertu/` owned by `alertu`, and
the daemon writes the file the first time anyone configures anything. The
commented example goes to `/usr/share/doc/alertu/config.example.toml` as
documentation, not as an active file.

Consequence: `dpkg` never prompts, and a fresh install starts on defaults with
no remote configured — which the daemon now says explicitly in its log.

## Package contents

```
/usr/bin/alertu-daemon
/usr/bin/alertu-ctl
/usr/bin/alertu-gui
/usr/bin/alertu-settings
/lib/systemd/system/alertu-daemon.service
/usr/lib/systemd/user/alertu-gui.service
/usr/lib/sysusers.d/alertu.conf
/usr/share/sounds/alertu/{beep,warning,siren}.wav
/usr/share/icons/hicolor/{48x48,64x64,128x128,256x256,512x512}/apps/alertu.png
/usr/share/applications/alertu-settings.desktop
/usr/share/doc/alertu/README.Debian
/usr/share/doc/alertu/config.example.toml
/usr/share/doc/alertu/copyright
```

Two paths differ from the manual install, and the README must document both:
the tray and settings window move from `~/.local/bin` to `/usr/bin`, and the
user unit from `~/.config/systemd/user` to `/usr/lib/systemd/user`. A package
installs for every account on the machine, not for one.

The sounds are **generated during the build** by `alertu-ctl gen-sounds` and
shipped as ordinary package files, so `dpkg` owns them and removal takes them
away. Generating them in a post-install script would leave orphans behind.

### The unit files hardcode the wrong prefix

`packaging/alertu-daemon.service` and `packaging/alertu-gui.service` both use
`/usr/local/bin`, which is correct for a manual install under the FHS and wrong
inside a package, which installs to `/usr/bin`.

`cargo-deb` cannot transform an asset, so the build patches the two units into a
staging directory before packaging. The committed files keep `/usr/local/bin`
for the manual path; the package ships the rewritten copies.

## Where the metadata lives

`cargo-deb` packages one crate, and our four binaries live in four crates. The
`[package.metadata.deb]` block therefore goes in
`crates/alertu-daemon/Cargo.toml`, with `name = "alertu"` so the package is not
called `alertu-daemon`, and the other three binaries declared as `assets`
pointing at `target/release/`.

This is a wart: one crate's manifest carries the whole project's packaging
configuration. The alternative — an empty `alertu` crate acting as a
metapackage — adds a phantom crate for a cosmetic gain. The wart is chosen and
documented rather than hidden.

Because the assets reference pre-built binaries, the build is two steps:
`cargo build --release` for the workspace, then `cargo deb --no-build`.

## Maintainer scripts

**postinst**: run `systemd-sysusers` so the `alertu` account exists with its
`input` and `video` groups; create `/etc/alertu/` owned by `alertu`; enable and
start `alertu-daemon`; refresh the icon cache; print the group instruction (see
below).

**prerm**: stop and disable the service.

**postrm purge**: remove `/etc/alertu/`.

**Purge deliberately does not remove `/var/lib/alertu/snapshots/`.** Debian
policy would allow it, and that is the expected behaviour for package-created
data. But those files are photographs of an intrusion — possibly the only record
of an incident. Destroying them because someone uninstalled a program is a bad
trade. Purge leaves them and prints where they are.

`cargo-deb` can generate the systemd enable/start fragments itself, from
`[package.metadata.deb.systemd-units]`, *and* accept hand-written maintainer
scripts. How the two combine — whether the generated snippets are appended to
ours via a `#DEBHELPER#` token, or replace them — is not verified. The first
implementation step is to find out and follow whichever mechanism the tool
actually supports, rather than assume. If they cannot be combined cleanly, the
scripts are written by hand in full and the generated units feature is left off.

## Dependencies

`Depends` is resolved automatically by `dpkg-shlibdeps`, which **only works when
the package is built on a Debian-based system**. Verified: running `cargo deb`
on Fedora emits `no dependency information found for /lib64/libgcc_s.so.1` and
produces a package declaring no dependencies at all. This is a correctness
argument for building in CI, not merely a convenience.

`Recommends`, for things the daemon degrades gracefully without — it warns and
carries on rather than failing:

- `ffmpeg | fswebcam` — webcam capture
- `pulseaudio-utils | alsa-utils` — audio playback

A headless machine can legitimately install neither.

## What cannot be automated

The control socket is `0660`. **A package cannot add a human to the `alertu`
group**: it does not know which account is theirs, `$SUDO_USER` is absent during
a non-interactive upgrade, and Debian policy forbids modifying existing
accounts.

Without that membership the tray, the settings window and `alertu-ctl` all fail
to connect — and it is the first thing anyone installing this package will hit.
So the post-install script prints the instruction, and `README.Debian` explains
it:

```
sudo usermod -aG alertu <user>   # then log out and back in
```

## Release workflow

A new workflow triggered on a `v*` tag, plus `workflow_dispatch` so the
packaging can be debugged without polluting the tag history.

1. Check the tag matches the version in `Cargo.toml`, so `v0.2.0` on a project
   still at `0.1.0` fails instead of producing a mislabelled package.
2. Build the workspace in release.
3. Generate the sounds, stage the patched unit files.
4. `cargo deb --no-build`.
5. **Install the result in the same job** — `dpkg -i`, then confirm the service
   starts and `alertu-ctl --version` answers. A package that builds but does not
   install is worthless, and that is only discovered by trying.
6. Attach the `.deb` to the GitHub release.

## Testing

- The install step in the release workflow is the primary test, and it runs on
  every tag.
- Locally, before pushing: build and install the package in a Debian container
  with `podman` (rootless, already available), confirming the file list, the
  service start, and that purge leaves the snapshot directory alone.
- The existing suite is untouched; this work adds no library code.

## Out of scope

- Splitting into `alertu` and `alertu-gui` packages.
- Inclusion in the Debian archive proper, which needs native `debian/` packaging
  with `debhelper` and `dh-cargo` rather than `cargo-deb`.
- An `.rpm`, despite the maintainer running Fedora.
- Publishing to crates.io.
