# Fedora RPM Package Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship AlertU as a single `alertu` RPM for Fedora, built by CI on tag and attached to the GitHub release alongside the existing `.deb`.

**Architecture:** `cargo-generate-rpm` metadata lives in `crates/alertu-daemon/Cargo.toml` beside the existing `[package.metadata.deb]` block, so the two file lists sit side by side and diverge visibly. The staging script that rewrites unit paths and generates sounds is shared by both formats. Scriptlets are hand-written shell — `cargo-generate-rpm` expands no RPM macros, so there is no `%systemd_post` and no `#DEBHELPER#` equivalent.

**Tech Stack:** cargo-generate-rpm 0.21, RPM scriptlets (POSIX sh), GitHub Actions with Fedora 43 containers, podman/docker for verification.

## Global Constraints

Every one of these was established by a review finding on the Debian branch, not by preference. They bind every task.

- **No file under `/etc` in the payload.** The daemon rewrites its own config whenever the tray, the settings window or `alertu-ctl set-config` saves; a packaged config file would be fought over by the package manager and the application. `/etc/alertu` is created by a scriptlet, owned by `alertu`, mode `0755`.
- **The package never adds anyone to the `alertu` group.** Socket membership confers full command of the alarm. The package prints the `usermod` instruction and stops.
- **The tray's user unit ships but is never enabled by the package.** A system-level scriptlet has no business touching a user's systemd instance. `systemctl --user enable --now alertu-gui` appears in the post-install message and in the README.
- **Sounds install mode `644`**, readable by the `alertu` service account. Binaries `755`, everything else `644`.
- **First-use guidance prints on fresh install only** (`$1 -eq 1`), never on upgrade.
- **No Rust source changes.** `crates/*/src/` is off limits. The only crate edit is `crates/alertu-daemon/Cargo.toml`.
- **Snapshots under `/var/lib/alertu/snapshots` survive removal.** On RPM this needs no code — files created at runtime by systemd's `StateDirectory` are not in the manifest — but it must be verified, not assumed.
- **Gates before every commit:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` (zero warnings), `cargo test --workspace --all-features --locked` (104 tests).
- **Architecture is `x86_64` only.**
- **Build the RPM inside a `fedora:43` container, never on Ubuntu.** Building the `.deb` on Fedora produced an empty `Depends` because `dpkg-shlibdeps` had no package database; the inverse holds for `--auto-req`. This is the branch's central lesson, in mirror.

## A deliberate divergence from the Debian package

The `.deb` enables and starts the daemon on install. This RPM does not, and that is intentional.

Fedora's convention is `systemctl preset`, which applies the administrator's preset policy. Fedora's default policy ends with `disable *`, so a third-party unit that appears in no preset file is left disabled. Using `enable --now` instead would override a policy the administrator may have set deliberately.

The consequence, which must be documented everywhere the install is described: **after `dnf install`, the daemon is not running.** The Fedora install is four commands, not three. This mirrors what Docker, Tailscale and most third-party Fedora packages do.

## File Structure

| File | Responsibility |
| --- | --- |
| `packaging/stage.sh` | Moved from `packaging/deb/stage.sh`. Rewrites unit paths `/usr/local/bin` → `/usr/bin` and generates the sounds. Now serves both formats; staging directory renamed `target/deb-staging` → `target/pkg-staging`. |
| `crates/alertu-daemon/Cargo.toml` | Gains `[package.metadata.generate-rpm]` and its `requires`/`recommends` sub-tables, after the existing deb block. |
| `packaging/rpm/post_install.sh` | `%post`. sysusers, `/etc/alertu`, `daemon-reload`, `preset` on fresh install, icon cache, first-use message. |
| `packaging/rpm/pre_uninstall.sh` | `%preun`. Disables and stops the daemon **only** on final removal. |
| `packaging/rpm/post_uninstall.sh` | `%postun`. `daemon-reload`; restarts the daemon on upgrade. |
| `packaging/rpm/README.Fedora` | Ships to `/usr/share/doc/alertu/`. Mirror of `README.Debian`, carrying the Fedora-specific enable step. |
| `.github/workflows/release.yml` | Gains an `rpm` job beside the existing `deb` job. |
| `README.md` | Gains a "Fedora" section beside "Debian and Ubuntu". |

---

### Task 1: Share the staging script between both formats

The script that rewrites unit paths and generates sounds is not Debian-specific — the RPM needs exactly the same two things. Move it up a level and rename its output directory before anything else depends on the old name.

**Files:**
- Move: `packaging/deb/stage.sh` → `packaging/stage.sh`
- Modify: `crates/alertu-daemon/Cargo.toml` (three `target/deb-staging` references)
- Modify: `.github/workflows/release.yml` (one `./packaging/deb/stage.sh` reference)

**Interfaces:**
- Consumes: nothing.
- Produces: `packaging/stage.sh`, which populates `target/pkg-staging/{system,user,sounds}/`. Tasks 2 and 4 depend on that path.

- [ ] **Step 1: Find every reference to the old paths**

```bash
cd "$(git rev-parse --show-toplevel)"
grep -rn "deb-staging\|packaging/deb/stage.sh" \
  --include='*.toml' --include='*.yml' --include='*.sh' --include='*.md' \
  . | grep -v '^./target/'
```

Expected: four hits — three in `crates/alertu-daemon/Cargo.toml`, one in `.github/workflows/release.yml`. If you find more (for example in `README.md`), update those too; the list below is not exhaustive by construction.

- [ ] **Step 2: Move the script and rename its staging directory**

```bash
git mv packaging/deb/stage.sh packaging/stage.sh
sed -i 's|target/deb-staging|target/pkg-staging|g' packaging/stage.sh
```

Then update the header comment in `packaging/stage.sh`, which currently says the two directories exist because "cargo-deb scans one directory for systemd units". Replace that paragraph with:

```sh
# The system unit and the user unit go to *separate* directories on purpose.
# cargo-deb scans one directory for systemd units and handles system units
# only; a user unit found there would be installed and enabled system-wide.
# cargo-generate-rpm does not scan at all — it takes an explicit asset list —
# but the split costs nothing and keeps one staging layout for both formats.
```

Also update the final `echo` and the `rm -rf`/`mkdir -p` lines if they still name `deb-staging` (the `sed` above should have caught them; verify).

- [ ] **Step 3: Update the two consumers**

In `crates/alertu-daemon/Cargo.toml`, three occurrences:

```toml
    ["../../target/pkg-staging/user/alertu-gui.service", "usr/lib/systemd/user/", "644"],
    ["../../target/pkg-staging/sounds/*.wav", "usr/share/sounds/alertu/", "644"],
```

and

```toml
unit-scripts = "../../target/pkg-staging/system"
```

In `.github/workflows/release.yml`:

```yaml
          ./packaging/stage.sh
```

- [ ] **Step 4: Verify the script still works and the deb still builds identically**

```bash
cargo build --release --locked
./packaging/stage.sh
ls target/pkg-staging/system target/pkg-staging/user target/pkg-staging/sounds
```

Expected: `alertu-daemon.service`, `alertu-gui.service`, and three `.wav` files. Then:

```bash
cargo deb --no-build -p alertu-daemon
```

Expected: writes `target/debian/alertu_0.1.0-1_amd64.deb` with no error. Confirm the payload is unchanged:

```bash
dpkg-deb -c target/debian/alertu_0.1.0-1_amd64.deb | awk '{print $6}' | sort > /tmp/after.txt
git stash && cargo build --release --locked -q && ./packaging/deb/stage.sh >/dev/null 2>&1
cargo deb --no-build -p alertu-daemon >/dev/null
dpkg-deb -c target/debian/alertu_0.1.0-1_amd64.deb | awk '{print $6}' | sort > /tmp/before.txt
git stash pop
diff /tmp/before.txt /tmp/after.txt && echo "payload identique"
```

Expected: `payload identique`. If `git stash` is awkward in your environment, compare against the released artefact instead:
`gh release download v0.1.0 -p '*.deb' -D /tmp/rel && dpkg-deb -c /tmp/rel/*.deb | awk '{print $6}' | sort | diff - /tmp/after.txt`

- [ ] **Step 5: Run the gates and commit**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
git add -A
git commit -m "Share the packaging staging script between formats

The script rewrites unit paths and generates sounds; neither is
Debian-specific, and the RPM needs both. Moved to packaging/stage.sh with its
output directory renamed target/pkg-staging.

Verified the .deb payload is byte-identical afterwards."
```

---

### Task 2: RPM metadata and assets

Produce an RPM with the correct payload and correctly resolved dependencies. No scriptlets yet — this task is about what lands on disk and what the package claims to need.

**Files:**
- Modify: `crates/alertu-daemon/Cargo.toml` (append after the `[package.metadata.deb.systemd-units]` block)

**Interfaces:**
- Consumes: `target/pkg-staging/` from Task 1.
- Produces: `target/generate-rpm/alertu-0.1.0-1.x86_64.rpm`. Tasks 3 and 4 build on it.

- [ ] **Step 1: Add the metadata block**

Append to `crates/alertu-daemon/Cargo.toml`. Note that `cargo-generate-rpm`'s asset syntax is inline tables with named fields, unlike cargo-deb's positional arrays — the two blocks look different because the tools are different, not because the packages differ.

```toml
# --- Fedora RPM -------------------------------------------------------------
#
# Deliberately mirrors [package.metadata.deb] above: same binaries, same paths,
# same sounds, same icons. Keep the two lists in step; if you add a file to one,
# add it to the other.
[package.metadata.generate-rpm]
name = "alertu"
summary = "Car-alarm-style intrusion guard for Linux desktops"
license = "MIT"
# Built and verified on Fedora 43. The effective floor is whatever glibc symbol
# versions --auto-req records, which is looser than a distribution version.
assets = [
    { source = "../../target/release/alertu-daemon",   dest = "/usr/bin/alertu-daemon",   mode = "755" },
    { source = "../../target/release/alertu-ctl",      dest = "/usr/bin/alertu-ctl",      mode = "755" },
    { source = "../../target/release/alertu-gui",      dest = "/usr/bin/alertu-gui",      mode = "755" },
    { source = "../../target/release/alertu-settings", dest = "/usr/bin/alertu-settings", mode = "755" },
    { source = "../../target/pkg-staging/system/alertu-daemon.service", dest = "/usr/lib/systemd/system/alertu-daemon.service", mode = "644" },
    { source = "../../target/pkg-staging/user/alertu-gui.service",      dest = "/usr/lib/systemd/user/alertu-gui.service",      mode = "644" },
    { source = "../../target/pkg-staging/sounds/*.wav", dest = "/usr/share/sounds/alertu/", mode = "644" },
    { source = "../../packaging/sysusers.d/alertu.conf", dest = "/usr/lib/sysusers.d/alertu.conf", mode = "644" },
    { source = "../../packaging/alertu-settings.desktop", dest = "/usr/share/applications/alertu-settings.desktop", mode = "644" },
    { source = "../../packaging/icons/hicolor/48x48/apps/alertu.png",   dest = "/usr/share/icons/hicolor/48x48/apps/alertu.png",   mode = "644" },
    { source = "../../packaging/icons/hicolor/64x64/apps/alertu.png",   dest = "/usr/share/icons/hicolor/64x64/apps/alertu.png",   mode = "644" },
    { source = "../../packaging/icons/hicolor/128x128/apps/alertu.png", dest = "/usr/share/icons/hicolor/128x128/apps/alertu.png", mode = "644" },
    { source = "../../packaging/icons/hicolor/256x256/apps/alertu.png", dest = "/usr/share/icons/hicolor/256x256/apps/alertu.png", mode = "644" },
    { source = "../../packaging/icons/hicolor/512x512/apps/alertu.png", dest = "/usr/share/icons/hicolor/512x512/apps/alertu.png", mode = "644" },
    { source = "../../packaging/config.example.toml", dest = "/usr/share/doc/alertu/config.example.toml", mode = "644", doc = true },
    { source = "../../packaging/rpm/README.Fedora",   dest = "/usr/share/doc/alertu/README.Fedora",      mode = "644", doc = true },
    { source = "../../LICENSE", dest = "/usr/share/licenses/alertu/LICENSE", mode = "644", doc = true },
]

# By file path, not by package name. /usr/bin/loginctl is provided by systemd;
# naming the path keeps the requirement about the capability rather than about
# one distribution's package layout.
[package.metadata.generate-rpm.requires]
"/usr/bin/loginctl" = "*"

# Weak dependencies: dnf installs them by default, the user may remove them,
# and an unresolvable one is silently ignored.
#
# The webcam entry is the important one. The daemon resolves its tool at
# runtime -- fswebcam if present, otherwise ffmpeg (snapshot.rs). On Fedora,
# /usr/bin/ffmpeg comes from ffmpeg-free in the Fedora repositories, while the
# package literally named `ffmpeg` lives in RPM Fusion. Requiring the name
# would force users onto a third-party repository; requiring the path does not.
#
# mesa-libGL and libxkbcommon are dlopen'ed by winit and glutin, so --auto-req
# cannot see them and the GUI would otherwise declare no runtime dependency.
[package.metadata.generate-rpm.recommends]
fswebcam = "*"
"/usr/bin/ffmpeg" = "*"
"/usr/bin/paplay" = "*"
mesa-libGL = "*"
libxkbcommon = "*"
```

- [ ] **Step 2: Create the placeholder README.Fedora so the asset resolves**

Task 4 writes the real content. An asset that does not exist makes `cargo generate-rpm` fail, so create the file now with content that is already true:

```bash
mkdir -p packaging/rpm
cat > packaging/rpm/README.Fedora <<'EOF'
AlertU on Fedora
================

See /usr/share/doc/alertu/config.example.toml for the configuration reference.
EOF
```

- [ ] **Step 3: Build the RPM inside a Fedora container**

The host is Fedora but has no `rpm-build`, and `--auto-req` silently degrades to emitting nothing without `/usr/lib/rpm/find-requires`. Build in the container that CI will use. SELinux is enforcing on this host, so the bind mount needs `:Z`.

```bash
cd "$(git rev-parse --show-toplevel)"
podman run --rm -v "$PWD":/w:Z -w /w fedora:43 bash -c '
  set -e
  dnf install -y -q gcc rust cargo rpm-build \
    libxkbcommon-devel wayland-devel mesa-libGL-devel \
    libX11-devel libXcursor-devel libXrandr-devel libXi-devel >/dev/null
  cargo install cargo-generate-rpm --locked -q
  cargo build --release --locked
  ./packaging/stage.sh
  ~/.cargo/bin/cargo-generate-rpm -p crates/alertu-daemon --auto-req auto
'
ls -l target/generate-rpm/
```

Expected: `target/generate-rpm/alertu-0.1.0-1.x86_64.rpm`.

If `cargo install cargo-generate-rpm` is slow enough to be painful while iterating, `dnf install -y rust-cargo-generate-rpm` may be available; check, and prefer it if so. Report which you used.

- [ ] **Step 4: Verify the payload and the dependencies**

```bash
RPM=target/generate-rpm/alertu-0.1.0-1.x86_64.rpm
rpm -qpl "$RPM"
rpm -qp --requires "$RPM"
rpm -qp --recommends "$RPM"
```

Assert all of the following, and report each explicitly:

1. The file list matches the deb's, path for path. Compare mechanically:
   ```bash
   rpm -qpl "$RPM" | sort > /tmp/rpm-files.txt
   dpkg-deb -c target/debian/alertu_0.1.0-1_amd64.deb \
     | awk '{print $6}' | sed 's|^\.||' | grep -v '/$' | sort > /tmp/deb-files.txt
   diff /tmp/deb-files.txt /tmp/rpm-files.txt
   ```
   Differences are expected and must each be justified in your report: the RPM
   carries `README.Fedora` where the deb carries `README.Debian`, the RPM
   carries `/usr/share/licenses/alertu/LICENSE` and no `changelog.Debian.gz`,
   and the deb's `copyright` file has no RPM equivalent. Any *other* difference
   is a defect.

2. `Requires` contains a `libc.so.6` symbol requirement. This is the assertion
   that catches a silently degraded `--auto-req` — the exact failure that
   shipped an empty `Depends` from Fedora on the Debian branch:
   ```bash
   rpm -qp --requires "$RPM" | grep -q 'libc\.so\.6' || echo "AUTO-REQ CASSÉ"
   ```

3. `Requires` contains `/usr/bin/loginctl`.

4. `Recommends` lists all five entries.

5. Measure the compatibility floor rather than asserting one. Unlike the deb's
   blunt `libc6 (>= 2.39)`, RPM records the individual glibc symbol versions
   actually referenced, which is a weaker and more accurate constraint:
   ```bash
   rpm -qp --requires "$RPM" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -3
   ```
   Record the highest version in your report. Task 4's README sentence is
   written from this number — do not guess it, and do not copy the Debian
   floor, which comes from a different mechanism.

- [ ] **Step 5: Check whether fswebcam actually exists in Fedora**

The spec flags this as unverified. Settle it:

```bash
podman run --rm fedora:43 bash -c 'dnf -q repoquery --qf "%{name} (%{reponame})" fswebcam 2>&1 | head -3'
```

If it resolves, nothing changes. If it does not, `/usr/bin/ffmpeg` becomes the effective default — say so in your report, because Task 4's README wording depends on it.

- [ ] **Step 6: Run the gates and commit**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
git add crates/alertu-daemon/Cargo.toml packaging/rpm/README.Fedora
git commit -m "Add RPM metadata mirroring the Debian package

Same binaries, units, sounds, icons and desktop entry. Dependencies are
declared by file path rather than package name: /usr/bin/ffmpeg is provided by
ffmpeg-free from Fedora proper, while the package named ffmpeg lives in RPM
Fusion, so requiring the name would push users onto a third-party repository."
```

---

### Task 3: Scriptlets and the full install/upgrade/remove lifecycle

This is where RPM's argument semantics differ from Debian's, and where a transcription of the Debian logic fails silently. The lifecycle test is the deliverable, not an afterthought.

**Files:**
- Create: `packaging/rpm/post_install.sh`, `packaging/rpm/pre_uninstall.sh`, `packaging/rpm/post_uninstall.sh`
- Modify: `crates/alertu-daemon/Cargo.toml` (`[package.metadata.generate-rpm]`: add the three script fields)

**Interfaces:**
- Consumes: the RPM from Task 2.
- Produces: an RPM whose service lifecycle is correct across install, upgrade and removal.

**The rule the scriptlets must obey.** In an RPM scriptlet, `$1` is the number of versions of the package that will remain installed once the operation finishes — it is not a verb as in a Debian maintainer script:

| Scriptlet | Fresh install | Upgrade | Final removal |
| --- | --- | --- | --- |
| `%post` | `1` | `2` | — |
| `%preun` | — | `1` | `0` |
| `%postun` | — | `1` | `0` |

The Debian `prerm` stops the unit unconditionally and relies on `postinst` to start it again. Transcribed unchanged, that produces a package which stops the daemon on **every upgrade**, with no error anywhere.

- [ ] **Step 1: Write `packaging/rpm/post_install.sh`**

```sh
#!/bin/sh
# %post. $1 is the number of versions of this package that will be installed
# once the operation finishes: 1 on a fresh install, 2 during an upgrade.
set -e

# The service account and its input/video groups. Shipped as a sysusers.d
# fragment so systemd owns the definition; this applies it now instead of
# waiting for the next boot.
systemd-sysusers /usr/lib/sysusers.d/alertu.conf >/dev/null 2>&1 || true

# No config file is shipped: anything under /etc would be a packaged config
# file, and the daemon rewrites this one whenever the tray or the CLI saves
# settings. It creates the file itself on first use; we only provide a
# directory it is allowed to write to.
mkdir -p /etc/alertu
chown alertu:alertu /etc/alertu 2>/dev/null || true
chmod 0755 /etc/alertu

systemctl daemon-reload >/dev/null 2>&1 || true

if [ "$1" -eq 1 ]; then
    # preset, not enable: it honours the administrator's preset policy, which
    # is the Fedora convention. Fedora's default policy ends with `disable *`,
    # so this leaves the daemon disabled and the user enables it -- see the
    # message below and README.Fedora.
    systemctl preset alertu-daemon.service >/dev/null 2>&1 || true
fi

if [ -x /usr/bin/gtk-update-icon-cache ]; then
    gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi

if [ "$1" -eq 1 ]; then
    cat <<'EOF'

AlertU has been installed.

The daemon is not running yet. Fedora applies preset policy rather than
enabling third-party units, so start it yourself:

    sudo systemctl enable --now alertu-daemon

Its control socket is mode 0660, owned by the `alertu` group. Connecting to it
grants full command of the alarm, so membership is a privilege grant -- which is
also why this package cannot grant it for you. Add yourself:

    sudo usermod -aG alertu <user>

then log out and back in. Until you do, the tray, the settings window and
alertu-ctl will not be able to reach the daemon.

The tray is shipped but not started: it is a user service, and a package must
not touch your systemd user instance. Once logged back in, run:

    systemctl --user enable --now alertu-gui

No remote is configured yet. Run `alertu-ctl list-devices`, then set
`remote_name_hint` to part of your remote's name. See
/usr/share/doc/alertu/config.example.toml.

EOF
fi
exit 0
```

- [ ] **Step 2: Write `packaging/rpm/pre_uninstall.sh`**

```sh
#!/bin/sh
# %preun. $1 is the number of versions that will remain after this operation:
# 0 on a real removal, 1 while an upgrade replaces this version.
#
# Guarding on 0 is the whole point. Stopping unconditionally -- which is what
# the Debian prerm does, because dpkg restarts the service afterwards -- would
# take the daemon down on every upgrade and never bring it back.
set -e

if [ "$1" -eq 0 ]; then
    systemctl --no-reload disable --now alertu-daemon.service >/dev/null 2>&1 || true
fi
exit 0
```

- [ ] **Step 3: Write `packaging/rpm/post_uninstall.sh`**

```sh
#!/bin/sh
# %postun. $1 is the number of versions remaining: 0 after a real removal,
# 1 or more after an upgrade.
set -e

systemctl daemon-reload >/dev/null 2>&1 || true

if [ "$1" -ge 1 ]; then
    # Upgrade: pick up the new binary. try-restart is a no-op when the unit is
    # not running, which is the common case here since Fedora leaves it
    # disabled unless the user enabled it.
    systemctl try-restart alertu-daemon.service >/dev/null 2>&1 || true
fi

if [ -x /usr/bin/gtk-update-icon-cache ]; then
    gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi

# /etc/alertu and /var/lib/alertu are deliberately left alone: the first holds
# the user's configuration, the second their captured snapshots. Neither is in
# the package manifest, so RPM does not touch them either.
exit 0
```

- [ ] **Step 4: Wire the scriptlets into the metadata**

Add to `[package.metadata.generate-rpm]` in `crates/alertu-daemon/Cargo.toml`, immediately after the `license` line:

```toml
post_install_script = "../../packaging/rpm/post_install.sh"
pre_uninstall_script = "../../packaging/rpm/pre_uninstall.sh"
post_uninstall_script = "../../packaging/rpm/post_uninstall.sh"
```

Verify the field names against `cargo generate-rpm --help` and the tool's documentation before assuming; if 0.21 expects the script *body* rather than a path, use the `*_script_path` variants if they exist, and report which form you used.

- [ ] **Step 5: Rebuild and confirm the scriptlets are embedded**

```bash
podman run --rm -v "$PWD":/w:Z -w /w fedora:43 bash -c '
  set -e
  dnf install -y -q gcc rust cargo rpm-build \
    libxkbcommon-devel wayland-devel mesa-libGL-devel \
    libX11-devel libXcursor-devel libXrandr-devel libXi-devel >/dev/null
  cargo install cargo-generate-rpm --locked -q
  cargo build --release --locked
  ./packaging/stage.sh
  ~/.cargo/bin/cargo-generate-rpm -p crates/alertu-daemon --auto-req auto
'
rpm -qp --scripts target/generate-rpm/alertu-0.1.0-1.x86_64.rpm
```

Expected: three scriptlets printed, each containing the `$1` guards above.

- [ ] **Step 6: The lifecycle test — this is the task's real deliverable**

A Fedora container with systemd as PID 1. Everything before this step is
inspection; this is the only step that can catch a wrong `$1`.

```bash
cd "$(git rev-parse --show-toplevel)"
cp target/generate-rpm/alertu-0.1.0-1.x86_64.rpm /tmp/alertu-test.rpm
chmod 644 /tmp/alertu-test.rpm

podman run -d --name alertu-rpm-test --privileged \
  -v /tmp/alertu-test.rpm:/tmp/a.rpm:ro,Z \
  fedora:43 /usr/sbin/init
sleep 5
podman exec alertu-rpm-test systemctl is-system-running --wait || true
```

If `/usr/sbin/init` is absent from the image, install systemd first
(`podman run ... fedora:43 bash -c 'dnf install -y systemd'` committed to a
local image, or use `registry.fedoraproject.org/fedora:43` which ships it).
Report what you had to do.

Then run each assertion and report its output verbatim:

```bash
X() { podman exec alertu-rpm-test bash -c "$1"; }

# 1. Install, and the first-use message appears.
X 'dnf install -y /tmp/a.rpm 2>&1 | grep -E "systemctl enable --now alertu-daemon|usermod|--user enable"'

# 2. Fedora leaves it disabled -- assert the documented behaviour, not a guess.
X 'systemctl is-enabled alertu-daemon; echo "is-enabled -> $?"'

# 3. The user enables it and the daemon actually runs as the service account
#    under ProtectSystem=strict.
X 'systemctl enable --now alertu-daemon && sleep 2 && systemctl is-active alertu-daemon'
X 'ps -o user,comm -C alertu-daemon'
X 'ls -l /run/alertu/alertu.sock'
```

Expected: `is-active` prints `active`; the process runs as `alertu`; the socket
is `srw-rw---- alertu alertu`.

```bash
# 4. Config persists despite ProtectSystem=strict.
X 'alertu-ctl set-config --grace-period-secs 20 && grep grace /etc/alertu/config.toml'

# 5. A snapshot directory to prove removal spares it.
X 'mkdir -p /var/lib/alertu/snapshots && touch /var/lib/alertu/snapshots/keep.jpg'

# 6. THE UPGRADE. This is the assertion the whole task exists for.
X 'dnf reinstall -y /tmp/a.rpm >/dev/null 2>&1; sleep 2; systemctl is-active alertu-daemon'
```

Expected: `active`. **If this prints `inactive`, the `$1` guards are wrong** —
that is precisely the silent failure this task guards against, and it must be
fixed before the task is complete.

```bash
# 7. Removal disables the unit and spares the user's data.
X 'dnf remove -y alertu >/dev/null 2>&1; systemctl is-active alertu-daemon; echo "---"; ls /var/lib/alertu/snapshots/; ls /etc/alertu/'
```

Expected: `inactive`; `keep.jpg` still present; `config.toml` still present.

```bash
podman rm -f alertu-rpm-test
```

- [ ] **Step 7: Run rpmlint and report, do not blindly fix**

```bash
podman run --rm -v /tmp/alertu-test.rpm:/tmp/a.rpm:ro,Z fedora:43 bash -c \
  'dnf install -y -q rpmlint >/dev/null 2>&1 && rpmlint /tmp/a.rpm'
```

Report every finding with a verdict: fixed, or accepted with a reason. The
Debian equivalent accepted several — `no-manual-page` among them — and that is
a legitimate outcome. Do not silence a finding you have not understood.

- [ ] **Step 8: Run the gates and commit**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
git add packaging/rpm crates/alertu-daemon/Cargo.toml
git commit -m "Add RPM scriptlets with correct \$1 semantics

In an RPM scriptlet \$1 counts the versions that will remain installed, where a
Debian maintainer script receives a verb. The Debian prerm stops the unit
unconditionally because dpkg restarts it afterwards; transcribed unchanged that
would stop the daemon on every RPM upgrade and never restart it.

Verified in a Fedora 43 container with systemd as PID 1: install, enable,
upgrade with the daemon still active afterwards, then removal sparing both
/etc/alertu and the captured snapshots."
```

---

### Task 4: CI job and documentation

**Files:**
- Modify: `.github/workflows/release.yml` (add an `rpm` job after the `deb` job)
- Rewrite: `packaging/rpm/README.Fedora`
- Modify: `README.md` (add a Fedora section after the "Debian and Ubuntu" section, which ends at the line `/usr/share/doc/alertu/README.Debian.`)

**Interfaces:**
- Consumes: everything from Tasks 1–3.
- Produces: an RPM attached to each release.

- [ ] **Step 1: Add the `rpm` job**

The job does **not** use `container:`. It builds inside a Fedora container it
launches itself, so that a second, systemd-enabled container can install and
verify the result — which a job running *inside* a container cannot do. This
mirrors the local protocol from Task 3 exactly.

Append to `.github/workflows/release.yml`:

```yaml
  rpm:
    name: Fedora package
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7

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

      # Built inside Fedora on purpose. --auto-req shells out to
      # /usr/lib/rpm/find-requires and resolves against the local RPM database;
      # on Ubuntu it would silently produce a package that claims to need
      # nothing. The Debian branch shipped exactly that failure in reverse.
      - name: Build and package inside Fedora
        run: |
          docker run --rm -v "$PWD":/w -w /w fedora:43 bash -c '
            set -eux
            dnf install -y -q gcc rust cargo rpm-build \
              libxkbcommon-devel wayland-devel mesa-libGL-devel \
              libX11-devel libXcursor-devel libXrandr-devel libXi-devel
            cargo install cargo-generate-rpm --locked
            cargo build --release --locked
            ./packaging/stage.sh
            ~/.cargo/bin/cargo-generate-rpm -p crates/alertu-daemon --auto-req auto
          '
          rpm=$(ls target/generate-rpm/*.rpm)
          echo "rpm=$rpm" >> "$GITHUB_ENV"
          echo "produced $rpm"

      - name: Show what was produced
        run: |
          docker run --rm -v "$PWD":/w -w /w fedora:43 bash -c "
            rpm -qpl '$rpm'
            rpm -qp --requires '$rpm'
            rpm -qp --recommends '$rpm'
            rpm -qp --requires '$rpm' | grep -q 'libc\.so\.6' || {
              echo 'Requires lost its --auto-req resolution' >&2; exit 1; }
          "

      # A package that builds but does not install is worthless, and that is
      # only discovered by trying. systemd as PID 1 so the unit is genuinely
      # enabled and started, and so the upgrade path is exercised -- the RPM
      # scriptlets' \$1 semantics fail silently and nothing else catches them.
      - name: Install, upgrade and remove it
        run: |
          docker run -d --name rpmtest --privileged \
            -v "$PWD/target/generate-rpm":/rpms:ro fedora:43 /usr/sbin/init
          sleep 5
          X() { docker exec rpmtest bash -c "$1"; }
          X 'dnf install -y /rpms/*.rpm'
          X 'alertu-ctl --version'
          X 'systemctl enable --now alertu-daemon'
          X 'sleep 2; systemctl is-active --quiet alertu-daemon' \
            || { docker exec rpmtest journalctl -u alertu-daemon --no-pager -n 50; exit 1; }
          X 'test -S /run/alertu/alertu.sock'
          X 'dnf reinstall -y /rpms/*.rpm'
          X 'sleep 2; systemctl is-active --quiet alertu-daemon' \
            || { echo "the daemon did not survive an upgrade: check the \$1 guards" >&2; exit 1; }
          X 'dnf remove -y alertu'
          docker rm -f rpmtest

      - uses: actions/upload-artifact@v4
        with:
          name: alertu-rpm
          path: ${{ env.rpm }}

      # `upload` requires a release to exist already, which it does not when the
      # tag was pushed with plain `git tag && git push --tags`. Create it if
      # missing, fall back to uploading into one made by hand or by the UI.
      - name: Attach to the release
        if: startsWith(github.ref, 'refs/tags/v')
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          gh release create "$GITHUB_REF_NAME" "$rpm" --generate-notes ||
            gh release upload "$GITHUB_REF_NAME" "$rpm" --clobber
```

- [ ] **Step 2: Validate the YAML parses**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml')); print('ok')"
```

Expected: `ok`. Confirm both jobs are present:

```bash
python3 -c "import yaml; print(list(yaml.safe_load(open('.github/workflows/release.yml'))['jobs']))"
```

Expected: `['deb', 'rpm']`.

- [ ] **Step 3: Write the real `packaging/rpm/README.Fedora`**

Mirror `packaging/deb/README.Debian` — read it first and follow its structure
and tone — with these Fedora-specific points:

- the daemon is **not** started by the package, and why (preset policy), with
  `sudo systemctl enable --now alertu-daemon`;
- the `usermod -aG alertu` step and why the package cannot do it;
- `systemctl --user enable --now alertu-gui` for the tray, and why a
  system-level scriptlet must not touch a user systemd instance;
- no configuration file is shipped; `/etc/alertu` is created empty and the
  daemon writes its own config;
- removal spares `/etc/alertu` and `/var/lib/alertu/snapshots`;
- the migration block for a previous manual install.

- [ ] **Step 4: Add the README section**

Insert after the Debian section (which ends `/usr/share/doc/alertu/README.Debian.`)
and before the line `Building from source, on any distribution:`.

Adjust the compatibility sentence to whatever Task 2 Step 4 actually measured,
and the webcam sentence to whatever Task 2 Step 5 found:

```markdown
### Fedora

An `.rpm` is attached to each [release](https://github.com/systm-d/alertU/releases),
named `alertu-<version>-1.x86_64.rpm`. It is built and verified on Fedora 43.

**If you installed manually before**, remove that install first — otherwise its
unit in `/etc/systemd/system` and its binaries in `/usr/local/bin` keep winning
over the packaged ones, and `dnf remove` will not remove either:

```sh
sudo systemctl disable --now alertu-daemon
sudo rm -f /etc/systemd/system/alertu-daemon.service
sudo rm -f /usr/local/bin/alertu-daemon /usr/local/bin/alertu-ctl
rm -f ~/.local/bin/alertu-gui ~/.local/bin/alertu-settings
rm -f ~/.config/systemd/user/alertu-gui.service
```

Then:

```sh
sudo dnf install ./alertu-*.x86_64.rpm
sudo systemctl enable --now alertu-daemon
sudo usermod -aG alertu "$USER"      # then log out and back in
systemctl --user enable --now alertu-gui
```

Unlike the `.deb`, the package does not start the daemon. Fedora applies preset
policy rather than enabling third-party units, and overriding that would
disregard a decision the administrator may have made deliberately — so the
second line is yours to run. Nor can a package add you to the `alertu` group,
or enable a unit in your user systemd instance. See
`/usr/share/doc/alertu/README.Fedora`.
```

- [ ] **Step 5: Confirm the documented commands match the shipped message**

Drift between the README and the post-install message is a real defect — the
Debian branch was caught with a README claiming the package "starts the daemon"
while the tray was left disabled and undocumented.

```bash
grep -n "enable --now" README.md packaging/rpm/README.Fedora packaging/rpm/post_install.sh
```

Every one of the three must name both `alertu-daemon` and `alertu-gui`, with
the same commands.

- [ ] **Step 6: Run the gates and commit**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
git add .github/workflows/release.yml README.md packaging/rpm/README.Fedora
git commit -m "Build and attach the RPM from CI, and document Fedora install

The job builds inside a Fedora container rather than using container: on the
job itself, so a second systemd-enabled container can install the result,
enable it, reinstall over it and assert the daemon is still active. That
upgrade assertion is the only thing that catches a wrong \$1 guard.

The README states plainly that the package does not start the daemon, which
diverges from the .deb and follows Fedora preset convention."
```

---

## Verification

After every task, and without exception at the end:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Expected: 104 tests pass, zero warnings. No test may be modified — this branch
changes no Rust source.

Specific to this plan:

```sh
# The RPM and the deb ship the same paths, modulo the documented exceptions.
rpm -qpl target/generate-rpm/alertu-*.rpm | sort > /tmp/rpm-files.txt
dpkg-deb -c target/debian/alertu_*.deb | awk '{print $6}' | sed 's|^\.||' \
  | grep -v '/$' | sort | diff - /tmp/rpm-files.txt

# --auto-req resolved something.
rpm -qp --requires target/generate-rpm/alertu-*.rpm | grep libc.so.6

# Nothing under /etc in the payload.
rpm -qpl target/generate-rpm/alertu-*.rpm | grep '^/etc' && echo DÉFAUT

# The daemon survives an upgrade (the lifecycle test from Task 3, Step 6).
```

And on this machine, once the branch is merged: install the RPM after running
the migration cleanup, and confirm `alertu-ctl status` and `alertu-ctl
list-devices` still work against the packaged daemon.

## Out of Scope

- COPR publication and source RPMs.
- A confined SELinux policy module.
- Architectures other than `x86_64`.
- Submission to Fedora proper.
- Any change under `crates/*/src/`.
