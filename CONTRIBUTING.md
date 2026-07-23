# Contributing to AlertU

Thanks for taking an interest. AlertU is a small, MIT-licensed personal
project, and contributions are welcome — from a typo fix to a new remote that
someone else's hardware needs.

## Before you start

- **For anything more than a small fix, open an issue first** and describe what
  you want to change. It is cheaper to agree on an approach in a paragraph than
  to review a branch that went the wrong way.
- **Security problems do not go in issues or pull requests.** Follow
  [`SECURITY.md`](SECURITY.md) — private reporting, so a fix ships before the
  detail is public.
- By contributing, you agree your work is licensed under the project's
  [MIT license](LICENSE). No separate CLA.

## Setting up

```sh
git clone https://github.com/systm-d/alertU
cd alertU
cargo build --release
```

The toolchain is pinned in `rust-toolchain.toml` (stable, with `rustfmt` and
`clippy`), so `rustup` fetches the right one automatically the first time you
build.

Only `alertu-settings` needs system libraries — its egui/eframe window links
X11, Wayland and GL. On Debian or Ubuntu:

```sh
sudo apt-get install -y --no-install-recommends \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libgl1-mesa-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev
```

If you would rather not install them, skip that crate:
`cargo build --release --workspace --exclude alertu-settings`.

## The checks that must pass

CI runs exactly these three, and a pull request will not go green until they do.
Run them locally before you push — they are fast:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Clippy is `-D warnings`: a warning fails the build. That is deliberate, so the
tree stays clean rather than accreting lint debt.

Two more checks run in CI and are worth knowing about, though you rarely need to
run them by hand:

- **MSRV.** The crate declares `rust-version = "1.88"`, and a CI job builds
  against exactly 1.88 to keep that promise honest. If you reach for a newer
  standard-library API, that job is where it surfaces.
- **Feature gating.** `alertu-common`'s IPC client sits behind an `ipc-client`
  feature that is off by default. CI builds `alertu-daemon` with default
  features (no `--all-features`) to confirm the gating still holds, so keep
  `ipc-client`-only code behind that flag.

## How the code is shaped

A couple of conventions are load-bearing; a reviewer will point you at them, so
here they are up front:

- **The daemon separates a pure decision table from its side effects.** The
  `decide` function in `crates/alertu-daemon/src/transitions.rs` maps a state
  and an event to a transition with no I/O; `machine.rs` calls it and then
  interprets the effects. New alarm behaviour usually means a case in the table
  plus a test on the pure part — no I/O required to test it.
- **Unsafe code is forbidden, with one documented exception.** Every crate
  carries `#![forbid(unsafe_code)]` except the daemon, which `#![deny]`s it
  crate-wide and scopes a single `#[allow(unsafe_code)]` to the `getgrnam_r`
  call in `perms.rs`. A pull request that adds `unsafe` elsewhere needs a real
  reason and a comment that carries its safety argument.
- **Match exhaustively; avoid `unreachable!`.** Prefer letting the compiler tell
  you when a new variant needs handling.

## Commits and pull requests

- Keep each commit focused on one change, and keep the working tree passing the
  three checks at every commit — not only at the end.
- Write the subject line as an imperative sentence describing the change
  (`Add a Fedora RPM package`, not `added rpm` or `feat: rpm`). Then use the
  body to explain **why**, and any consequence a future reader would not guess.
  Look at `git log` for the house style.
- If your change is user-visible, add a line under `## [Unreleased]` in
  [`CHANGELOG.md`](CHANGELOG.md).
- Open the pull request against `main`, say what you changed and how you tested
  it, and link the issue if there is one.

## If you are adding hardware support

AlertU hardcodes no device model — a remote is matched by a name substring and a
key name from your config. So "my remote does not work" is usually a
configuration question, not a code change: run `alertu-ctl list-devices`, then
set `remote_name_hint` and `toggle_keys`. If something genuinely cannot be
expressed in config, that is worth an issue.
