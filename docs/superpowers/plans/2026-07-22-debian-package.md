# Debian Package Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a `.deb` for Debian and Ubuntu users, built in CI and attached to GitHub releases.

**Architecture:** `cargo-deb` metadata lives in `crates/alertu-daemon/Cargo.toml` with the package renamed to `alertu`; a staging script prepares the two things that cannot be shipped as-is — unit files whose `ExecStart` points at `/usr/local/bin`, and the sound files, which are generated. Hand-written maintainer scripts carry the `#DEBHELPER#` token so cargo-deb can splice its systemd fragments in.

**Tech Stack:** cargo-deb 3.7, podman (verification), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-07-22-debian-package-design.md`

## Global Constraints

- One package, `alertu`, containing all four binaries. Not split.
- **No configuration file is shipped.** `/etc/alertu/` is created empty by the post-install script and owned by `alertu`; the daemon writes the file itself on first `SetConfig`. Anything shipped under `/etc` would be treated as a conffile automatically by cargo-deb, which is exactly what this avoids.
- **Purge must not remove `/var/lib/alertu/snapshots/`.** Those are photographs of an intrusion. Purge removes `/etc/alertu/` and prints where the snapshots are.
- No Rust source changes. This work adds no library code and must not modify `crates/*/src/`.
- The existing suite must stay green: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` (zero warnings), `cargo test --workspace --all-features --locked` (102 tests).
- Work on branch `claude/deb-packaging`. Do not push.

## Facts established before writing this plan

Verified against cargo-deb's own documentation and a probe run — do not re-derive:

- `cargo deb --no-build -p alertu-daemon` already works on this repository and produces a package. On Fedora it warns `No $auto deps`, because `dpkg-shlibdeps` has no Debian package database to consult. **Automatic dependency resolution only works when building on a Debian-based system**, which is why the release build happens in CI.
- `maintainer-scripts` **must** be set, even to an empty directory, for `systemd-units` to work. Any script present **must** contain the `#DEBHELPER#` token; cargo-deb splices its generated fragments at that point, or creates the script if absent.
- Unit files are matched by name against `<package>.<suffix>` or, when `unit-name` is given, `<unit>.<suffix>`. Our file is `alertu-daemon.service` and the package is `alertu`, so **`unit-name = "alertu-daemon"` is required** or nothing matches.
- **cargo-deb's systemd support handles system units only** — its documentation never mentions user units. `alertu-gui.service` must therefore be shipped as a plain asset and kept *out* of the `unit-scripts` directory, or it would be installed and enabled as a system unit.
- Files shipped under `/etc` are treated as conffiles automatically, without being listed in `conf-files`.

## File Structure

| File | Responsibility |
| --- | --- |
| `packaging/deb/stage.sh` (create) | Build the staging tree: patched units, generated sounds |
| `packaging/deb/postinst` (create) | sysusers, `/etc/alertu`, icon cache, the group message |
| `packaging/deb/prerm` (create) | `#DEBHELPER#` only — cargo-deb supplies the stop/disable |
| `packaging/deb/postrm` (create) | Purge `/etc/alertu`, keep the snapshots, say so |
| `packaging/deb/README.Debian` (create) | The group requirement, in prose |
| `crates/alertu-daemon/Cargo.toml` (modify) | `[package.metadata.deb]` |
| `.github/workflows/release.yml` (create) | Tag → build → install-test → attach |
| `README.md` (modify) | Point Debian users at the package |

---

### Task 1: Package metadata and contents

The deliverable is a `.deb` whose file list is exactly right. Maintainer scripts come next; this task is about *what is in the archive*.

**Files:**
- Create: `packaging/deb/stage.sh`
- Modify: `crates/alertu-daemon/Cargo.toml`

**Interfaces:**
- Consumes: nothing.
- Produces: a staging tree at `target/deb-staging/` with `system/alertu-daemon.service`, `user/alertu-gui.service`, `sounds/{beep,warning,siren}.wav`; and `[package.metadata.deb]` in `crates/alertu-daemon/Cargo.toml` naming the package `alertu`.

- [ ] **Step 1: Write the staging script**

Create `packaging/deb/stage.sh`:

```bash
#!/bin/bash
# Prepare the files that cannot be packaged as they are committed.
#
# Two of them:
#   * the systemd units hardcode /usr/local/bin, correct for a manual install
#     under the FHS and wrong in a package, which installs to /usr/bin;
#   * the sounds are generated, not committed.
#
# The system unit and the user unit go to *separate* directories on purpose.
# cargo-deb scans one directory for systemd units and handles system units
# only; a user unit found there would be installed and enabled system-wide.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
stage="$root/target/deb-staging"

rm -rf "$stage"
mkdir -p "$stage/system" "$stage/user" "$stage/sounds"

sed 's|/usr/local/bin|/usr/bin|g' \
    "$root/packaging/alertu-daemon.service" > "$stage/system/alertu-daemon.service"
sed 's|/usr/local/bin|/usr/bin|g' \
    "$root/packaging/alertu-gui.service" > "$stage/user/alertu-gui.service"

# Generated rather than committed, so dpkg owns them and removal takes them away.
"$root/target/release/alertu-ctl" gen-sounds --dir "$stage/sounds" --force

grep -q /usr/local/bin "$stage"/system/*.service "$stage"/user/*.service &&
  { echo "stage.sh: a unit still points at /usr/local/bin" >&2; exit 1; }

echo "staged into $stage"
```

Make it executable: `chmod +x packaging/deb/stage.sh`.

- [ ] **Step 2: Add the package metadata**

Append to `crates/alertu-daemon/Cargo.toml`:

```toml
# Debian packaging. It lives in this crate's manifest because cargo-deb packages
# one crate, while AlertU's four binaries live in four — so this one manifest
# carries the whole project's packaging configuration, and `name` renames the
# result from "alertu-daemon" to "alertu". A wart, chosen over adding an empty
# metapackage crate purely for cosmetics.
#
# Paths are relative to this manifest's directory.
[package.metadata.deb]
name = "alertu"
maintainer = "Kevin Delfour <kevin@delfour.co>"
license-file = ["../../LICENSE", "0"]
extended-description = """
A cheap USB or Bluetooth HID remote becomes the key fob for a Linux desktop.
Click to arm: the session locks. Activity on a watched input device then starts
a countdown, a siren, a timestamped webcam still and an optional webhook.
A personal gadget, not an anti-theft system."""
section = "utils"
priority = "optional"
depends = "$auto, systemd"
recommends = "ffmpeg | fswebcam, pulseaudio-utils | alsa-utils"
assets = [
    ["../../target/release/alertu-daemon",   "usr/bin/",  "755"],
    ["../../target/release/alertu-ctl",      "usr/bin/",  "755"],
    ["../../target/release/alertu-gui",      "usr/bin/",  "755"],
    ["../../target/release/alertu-settings", "usr/bin/",  "755"],
    ["../../target/deb-staging/user/alertu-gui.service", "usr/lib/systemd/user/", "644"],
    ["../../target/deb-staging/sounds/*.wav", "usr/share/sounds/alertu/", "644"],
    ["../../packaging/sysusers.d/alertu.conf", "usr/lib/sysusers.d/", "644"],
    ["../../packaging/alertu-settings.desktop", "usr/share/applications/", "644"],
    ["../../packaging/icons/hicolor/48x48/apps/alertu.png",   "usr/share/icons/hicolor/48x48/apps/",   "644"],
    ["../../packaging/icons/hicolor/64x64/apps/alertu.png",   "usr/share/icons/hicolor/64x64/apps/",   "644"],
    ["../../packaging/icons/hicolor/128x128/apps/alertu.png", "usr/share/icons/hicolor/128x128/apps/", "644"],
    ["../../packaging/icons/hicolor/256x256/apps/alertu.png", "usr/share/icons/hicolor/256x256/apps/", "644"],
    ["../../packaging/icons/hicolor/512x512/apps/alertu.png", "usr/share/icons/hicolor/512x512/apps/", "644"],
    ["../../packaging/config.example.toml", "usr/share/doc/alertu/", "644"],
    ["../../packaging/deb/README.Debian",   "usr/share/doc/alertu/", "644"],
]
maintainer-scripts = "../../packaging/deb"

# `unit-name` is required: units are matched as `<unit>.<suffix>`, and our file
# is alertu-daemon.service while the package is alertu, so the default
# `<package>.<suffix>` pattern would match nothing.
#
# `unit-scripts` points at the system-only staging directory. cargo-deb handles
# system units only; the user unit is shipped through `assets` above and must
# never appear here.
[package.metadata.deb.systemd-units]
unit-name = "alertu-daemon"
unit-scripts = "../../target/deb-staging/system"
enable = true
start = true
restart-after-upgrade = true
```

**The `maintainer` address** is taken from the author's own project template
(`../alertU-1/Cargo.toml`). Debian's control format requires `Name <email>`, and
the workspace's `authors = ["Systm D"]` has no address, so it cannot be derived.
Flag it in your report so the owner can change it before anything is published.

- [ ] **Step 3: Create the maintainer-scripts directory and its README.Debian**

`packaging/deb/` must exist for `maintainer-scripts` to resolve — cargo-deb
requires the directory even when empty. Git does not track empty directories, so
this task also writes the file that documents the package's two deliberate
omissions. Its content depends only on decisions already taken in the spec, not
on the scripts Task 2 adds.

```bash
mkdir -p packaging/deb
```

Create `packaging/deb/README.Debian`:

```
AlertU for Debian
=================

Two things this package deliberately does not do.

It does not add you to the `alertu` group.
-----------------------------------------

The daemon's control socket is mode 0660, owned by `alertu`. Anyone who can
connect to it can disarm the alarm, read the configuration including the
webhook URL, and redirect the paths handed to the helper programs. Group
membership is therefore a privilege grant, not a convenience.

A package cannot grant it: it does not know which account is yours, $SUDO_USER
is absent during a non-interactive upgrade, and Debian policy forbids modifying
existing accounts. So:

    sudo usermod -aG alertu <user>

then log out and back in. Without it, the tray, the settings window and
alertu-ctl will all fail to connect.

It does not ship a configuration file.
--------------------------------------

The daemon rewrites its configuration whenever you change a setting from the
tray, the settings window or `alertu-ctl set-config`. A file under /etc would be
a conffile, and dpkg would ask you to resolve a conflict on every upgrade.

Instead the package creates /etc/alertu/ and lets the daemon write
config.toml there itself. A commented example is in this directory. The daemon
starts on built-in defaults, with no remote configured, and says so in its log.

Removing the package
--------------------

`apt remove` leaves your configuration; `apt purge` deletes /etc/alertu.

Neither touches /var/lib/alertu/snapshots. Those are webcam stills from alarms
you may still want. Remove them by hand.
```

- [ ] **Step 4: Build and stage**

Run:

```bash
cargo build --release --locked
./packaging/deb/stage.sh
```

Expected: `staged into …/target/deb-staging`, and three WAVs plus two patched units on disk.

- [ ] **Step 5: Verify the units were actually patched**

Run: `grep -h ExecStart target/deb-staging/*/*.service`
Expected: both lines say `/usr/bin/`, neither says `/usr/local/bin`.

- [ ] **Step 6: Build the package and inspect its contents**

Run:

```bash
cargo deb --no-build -p alertu-daemon -o target/alertu.deb
dpkg-deb -c target/alertu.deb | awk '{print $6}' | sort
```

Every one of these must be present:

```
./usr/bin/alertu-ctl
./usr/bin/alertu-daemon
./usr/bin/alertu-gui
./usr/bin/alertu-settings
./usr/lib/sysusers.d/alertu.conf
./usr/lib/systemd/user/alertu-gui.service
./usr/share/applications/alertu-settings.desktop
./usr/share/doc/alertu/README.Debian
./usr/share/doc/alertu/config.example.toml
./usr/share/icons/hicolor/128x128/apps/alertu.png
./usr/share/icons/hicolor/256x256/apps/alertu.png
./usr/share/icons/hicolor/48x48/apps/alertu.png
./usr/share/icons/hicolor/512x512/apps/alertu.png
./usr/share/icons/hicolor/64x64/apps/alertu.png
./usr/share/sounds/alertu/beep.wav
./usr/share/sounds/alertu/siren.wav
./usr/share/sounds/alertu/warning.wav
./lib/systemd/system/alertu-daemon.service
```

cargo-deb adds files of its own — `copyright` certainly, and possibly a
compressed changelog. Those are expected; do not try to remove them.

The two paths worth checking individually, because getting either wrong is
silent rather than loud:

- `./lib/systemd/system/alertu-daemon.service` proves the `systemd-units` table
  matched. If it is missing, `unit-name` or `unit-scripts` is wrong and the
  service will never be installed or enabled.
- `./usr/lib/systemd/user/alertu-gui.service` must be **under `usr/lib/…/user/`**
  and must *not* also appear under `lib/systemd/system/`. If it turns up as a
  system unit, the user unit leaked into the `unit-scripts` directory.

If any asset path is missing entirely, its source path is wrong — cargo-deb
resolves them relative to the manifest's directory, so they start `../../`.

- [ ] **Step 7: Check the metadata and that nothing lands in /etc**

Run: `dpkg-deb -I target/alertu.deb`
Expected: `Package: alertu`, `Version: 0.1.0-1`, the recommends line, and the extended description.

Run: `dpkg-deb -c target/alertu.deb | grep -c '/etc/'`
Expected: `0`. Anything under `/etc` would become a conffile automatically and reintroduce the upgrade prompt this design exists to avoid.

- [ ] **Step 8: Verify the workspace is untouched**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features --locked -- -D warnings && cargo test --workspace --all-features --locked`
Expected: clean, 102 tests. No Rust source was edited.

- [ ] **Step 9: Commit**

```bash
git add packaging/deb crates/alertu-daemon/Cargo.toml
git commit -m "build(deb): package metadata and staging"
```

---

### Task 2: Maintainer scripts, verified in a container

**Files:**
- Create: `packaging/deb/postinst`, `packaging/deb/prerm`, `packaging/deb/postrm`

**Interfaces:**
- Consumes: the metadata from Task 1, which sets `maintainer-scripts = "../../packaging/deb"`.
- Produces: a package that installs, starts and purges cleanly.

- [ ] **Step 1: Write postinst**

Create `packaging/deb/postinst`:

```bash
#!/bin/sh
set -e

case "$1" in
configure)
    # The service account and its input/video groups. Shipped as a
    # sysusers.d fragment so systemd owns the definition; this just applies it
    # now instead of waiting for the next boot.
    if [ -x /usr/bin/systemd-sysusers ] || [ -x /bin/systemd-sysusers ]; then
        systemd-sysusers /usr/lib/sysusers.d/alertu.conf >/dev/null 2>&1 || true
    fi

    # No config file is shipped: anything under /etc would become a conffile,
    # and the daemon rewrites this one whenever the tray or the CLI saves
    # settings. It creates the file itself on first use; we only provide a
    # directory it is allowed to write to.
    mkdir -p /etc/alertu
    chown alertu:alertu /etc/alertu 2>/dev/null || true
    chmod 0755 /etc/alertu

    if [ -x /usr/bin/gtk-update-icon-cache ]; then
        gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || true
    fi

    cat <<'EOF'

AlertU is installed and its daemon is running.

Its control socket is mode 0660, owned by the `alertu` group. Connecting to it
grants full command of the alarm, so membership is a privilege grant — which is
also why this package cannot grant it for you. Add yourself:

    sudo usermod -aG alertu <user>

then log out and back in. Until you do, the tray, the settings window and
alertu-ctl will not be able to reach the daemon.

No remote is configured yet. Run `alertu-ctl list-devices`, then set
`remote_name_hint` to part of your remote's name. See
/usr/share/doc/alertu/config.example.toml.

EOF
    ;;
esac

#DEBHELPER#

exit 0
```

- [ ] **Step 2: Write prerm and postrm**

Create `packaging/deb/prerm`:

```bash
#!/bin/sh
set -e

#DEBHELPER#

exit 0
```

cargo-deb generates the stop and disable fragments; nothing else is needed here.

Create `packaging/deb/postrm`:

```bash
#!/bin/sh
set -e

case "$1" in
purge)
    # The daemon writes its config here; the package never shipped one.
    rm -rf /etc/alertu

    # Snapshots are deliberately kept. Debian policy would allow removing
    # package-created data, but these are photographs of an intrusion —
    # possibly the only record of an incident. Deleting them because someone
    # uninstalled a program is the wrong trade.
    if [ -d /var/lib/alertu/snapshots ]; then
        echo "AlertU: alarm snapshots kept in /var/lib/alertu/snapshots — remove them yourself if you want them gone."
    fi
    ;;
esac

#DEBHELPER#

exit 0
```

Make all three executable: `chmod +x packaging/deb/postinst packaging/deb/prerm packaging/deb/postrm`.

- [ ] **Step 3: Check the scripts and README.Debian agree**

`packaging/deb/README.Debian` was written in Task 1 and promises specific
behaviour. Read it against the three scripts you just wrote and confirm each
claim holds: purge removes `/etc/alertu`, purge leaves
`/var/lib/alertu/snapshots`, and no configuration file is shipped. A document
that describes behaviour the scripts do not have is worse than none.

- [ ] **Step 4: Rebuild the package**

Run:

```bash
cargo build --release --locked && ./packaging/deb/stage.sh
cargo deb --no-build -p alertu-daemon -o target/alertu.deb
dpkg-deb -I target/alertu.deb postinst | head -20
```

Expected: the postinst content, with cargo-deb's systemd fragments spliced in where `#DEBHELPER#` was — you should see `deb-systemd-helper` or `systemctl` lines that you did not write.

If `#DEBHELPER#` is still present verbatim, the `systemd-units` table is not being applied; check `unit-name` and `unit-scripts`.

- [ ] **Step 5: Install it in a Debian container**

The package cannot be installed on this Fedora machine, and `dpkg-shlibdeps` produced no dependency list here, so this run also shows what the package does on a real Debian system.

Run:

```bash
podman run --rm -v "$PWD/target:/pkg:ro" debian:stable-slim bash -c '
  set -e
  apt-get update -qq
  # Install the package and let apt resolve what dpkg reports as missing.
  dpkg -i /pkg/alertu.deb || apt-get -y -qq --fix-broken install
  echo "--- files ---"
  dpkg -L alertu | sort
  echo "--- the binaries run ---"
  alertu-ctl --version
  alertu-daemon --help | head -1
  echo "--- the service account exists ---"
  id alertu
  echo "--- /etc/alertu ---"
  ls -ld /etc/alertu
  echo "--- purge ---"
  apt-get -y -qq purge alertu
  ls /etc/alertu 2>&1 || echo "/etc/alertu removed, as intended"
'
```

Expected: the file list matches Task 1, both binaries answer, `id alertu` shows the account, `/etc/alertu` exists and is owned by `alertu`, and purge removes it.

**The service will not start in this container** — `debian:stable-slim` has no running systemd, so the postinst's start fragment fails or is skipped. That is expected and is not a package defect. Report exactly what happened rather than glossing over it; verifying an actual service start needs a systemd-capable container, which is out of scope here and is covered by the release workflow running on a real systemd host.

- [ ] **Step 6: Verify the snapshot directory survives purge**

Run:

```bash
podman run --rm -v "$PWD/target:/pkg:ro" debian:stable-slim bash -c '
  set -e
  apt-get update -qq
  dpkg -i /pkg/alertu.deb || apt-get -y -qq --fix-broken install
  mkdir -p /var/lib/alertu/snapshots
  echo evidence > /var/lib/alertu/snapshots/alertu-test.jpg
  apt-get -y -qq purge alertu
  test -f /var/lib/alertu/snapshots/alertu-test.jpg &&
    echo "PASS: the snapshot survived purge" ||
    { echo "FAIL: purge destroyed the snapshots"; exit 1; }
'
```

Expected: `PASS`. This is the one behaviour that deliberately departs from Debian convention, so it gets its own check.

- [ ] **Step 7: Commit**

```bash
git add packaging/deb
git commit -m "build(deb): maintainer scripts, verified in a Debian container"
```

---

### Task 3: Release workflow

**Files:**
- Create: `.github/workflows/release.yml`
- Modify: `README.md`

**Interfaces:**
- Consumes: `packaging/deb/stage.sh` and the metadata from Tasks 1–2.
- Produces: a `.deb` attached to the GitHub release for a `v*` tag.

- [ ] **Step 1: Write the workflow**

Create `.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags: ["v*"]
  workflow_dispatch:

permissions:
  contents: write

jobs:
  deb:
    name: Debian package
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7

      - uses: dtolnay/rust-toolchain@stable

      - name: Cache cargo
        uses: Swatinem/rust-cache@v2

      # alertu-settings builds eframe, which needs the X11/Wayland/GL headers.
      - name: Install build dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
            libgl1-mesa-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev

      # A tag that disagrees with Cargo.toml would produce a package labelled
      # with one version and containing another.
      - name: Check the tag matches the crate version
        if: startsWith(github.ref, 'refs/tags/v')
        run: |
          crate=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
          tag="${GITHUB_REF_NAME#v}"
          if [ "$crate" != "$tag" ]; then
            echo "tag $tag does not match Cargo.toml version $crate" >&2
            exit 1
          fi
          echo "version $crate"

      - uses: taiki-e/install-action@v2
        with:
          tool: cargo-deb

      - name: Build
        run: cargo build --release --locked

      - name: Stage and package
        run: |
          ./packaging/deb/stage.sh
          cargo deb --no-build -p alertu-daemon -o target/alertu.deb

      # Unlike the developer's Fedora machine, this runner has a Debian package
      # database, so `$auto` resolves real dependencies here — and this is the
      # only place that can be checked.
      - name: Show what was produced
        run: |
          dpkg-deb -I target/alertu.deb
          dpkg-deb -c target/alertu.deb

      # A package that builds but does not install is worthless, and that is
      # only ever discovered by trying. The runner has systemd, so the unit is
      # genuinely enabled and started here.
      - name: Install it
        run: |
          sudo apt-get install -y ./target/alertu.deb
          alertu-ctl --version
          systemctl is-enabled alertu-daemon
          sudo systemctl status alertu-daemon --no-pager --lines=0 || true
          dpkg -L alertu | sort

      - name: Remove it again
        run: sudo apt-get purge -y alertu

      - uses: actions/upload-artifact@v4
        with:
          name: alertu-deb
          path: target/alertu.deb

      - name: Attach to the release
        if: startsWith(github.ref, 'refs/tags/v')
        env:
          GH_TOKEN: ${{ github.token }}
        run: gh release upload "$GITHUB_REF_NAME" target/alertu.deb --clobber
```

- [ ] **Step 2: Validate the YAML**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml')); print('ok')"`
Expected: `ok`.

- [ ] **Step 3: Check the version-matching logic locally**

The workflow's check is a shell snippet; run its core against the real manifest:

```bash
sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1
```

Expected: `0.1.0`. A tag would therefore have to be `v0.1.0`.

- [ ] **Step 4: Document the package in the README**

In `README.md`, immediately before the `## Install (systemd)` heading, add:

```markdown
### Debian and Ubuntu

A `.deb` is attached to each [release](https://github.com/systm-d/alertU/releases):

```sh
sudo apt install ./alertu.deb
sudo usermod -aG alertu "$USER"   # then log out and back in
```

The package installs all four binaries, the systemd units, the sounds and the
desktop entry, and starts the daemon. It deliberately ships no configuration
file — the daemon writes its own on first use — and it cannot add you to the
`alertu` group, which is why that second line is not optional. See
`/usr/share/doc/alertu/README.Debian`.

Building from source, on any distribution:
```

- [ ] **Step 5: Verify the workspace is still clean**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features --locked -- -D warnings && cargo test --workspace --all-features --locked`
Expected: clean, 102 tests.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/release.yml README.md
git commit -m "ci: build and attach a .deb on release tags"
```

---

## Verification

| Check | Command | Expected |
| --- | --- | --- |
| Package builds | `cargo deb --no-build -p alertu-daemon -o target/alertu.deb` | succeeds |
| Name and version | `dpkg-deb -I target/alertu.deb \| grep Package` | `alertu` |
| Nothing in /etc | `dpkg-deb -c target/alertu.deb \| grep -c '/etc/'` | `0` |
| Fragments spliced | `dpkg-deb -I target/alertu.deb postinst \| grep -c DEBHELPER` | `0` |
| Installs in a container | Task 2 Step 5 | file list, binaries answer, account exists |
| Purge keeps evidence | Task 2 Step 6 | `PASS` |
| Workspace untouched | `git diff main --stat -- crates/*/src` | empty |
| Suite green | `cargo test --workspace --all-features --locked` | 102 tests |

## Out of scope

- Splitting into `alertu` and `alertu-gui` packages.
- Inclusion in the Debian archive, which needs native `debian/` packaging with `debhelper` and `dh-cargo`.
- An `.rpm`.
- Publishing to crates.io.
- Actually tagging a release — the workflow exists, pressing the button is the owner's call.
