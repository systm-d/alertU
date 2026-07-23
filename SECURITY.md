# Security Policy

## What AlertU is, and what it is not

AlertU is a personal gadget: a USB or Bluetooth remote arms a guard that locks
your desktop session, and activity on a watched input device trips a countdown,
a siren, a timestamped webcam still and an optional webhook. It is a deterrent
and a notifier, **not an anti-theft system**.

Anyone with physical access can cut the power, unplug the machine, or — if they
hold your login — disarm it. No desktop software can prevent that, and AlertU
does not claim to. Keep that in mind both when you rely on it and when you
decide whether something is a vulnerability.

## Supported versions

Fixes land on the latest release only. AlertU is `0.x`; there are no backports
to earlier tags.

| Version | Supported |
| ------- | --------- |
| 0.2.x   | ✅        |
| < 0.2   | ❌        |

## The security model

Understanding these boundaries tells you what is a bug and what is the design.

- **The control socket** (`/run/alertu/alertu.sock`) is mode `0660`, owned by
  the `alertu` group. Connecting to it grants full command of the alarm — arm,
  disarm, and rewrite the daemon's configuration. Group membership is therefore
  a privilege grant, which is exactly why installing the package does not add
  you to the group: you do that yourself, deliberately. A member of the `alertu`
  group disarming the alarm is the model working as intended, not a flaw.

- **Snapshots** (`/var/lib/alertu/snapshots`, directory `0750`, files `0640`)
  are photographs of whoever is at the machine. They are deliberately not
  world-readable. A path that let an unprivileged user read them, or that let
  the daemon write them somewhere world-readable, would be a real issue.

- **The daemon** runs as a dedicated `alertu` service account, never root, with
  only the `input` and `video` supplementary groups it needs, under systemd
  hardening (`NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`,
  `PrivateTmp`). It writes only to `/etc/alertu` and `/var/lib/alertu`.

- **Unsafe code** is confined to a single `libc::getgrnam_r` call in
  `crates/alertu-daemon/src/perms.rs`, needed to resolve a group by name. The
  daemon denies unsafe crate-wide and scopes one `#[allow(unsafe_code)]` to that
  module; every other crate carries `#![forbid(unsafe_code)]` outright.

- **No network listener.** The only IPC surface is the local Unix socket. The
  webhook is outbound only, to a URL you configure, fired via `curl` on alarm.

## In scope

Reports where the daemon steps outside these boundaries are welcome, in
particular:

- privilege escalation — the `alertu` account gaining more than it should, or an
  unprivileged local user gaining the group's powers without membership;
- the control socket, snapshots, or the config becoming reachable beyond their
  documented permissions;
- config or path handling that makes the daemon create, `chmod`, `chown`, or
  delete files outside the directories it owns;
- memory-safety problems reachable through the one unsafe block or through
  crafted input events.

## Out of scope

- Physical access — powering off, unplugging, or booting other media.
- Anyone already in the `alertu` group disarming the alarm or reading snapshots;
  that is the documented privilege boundary.
- The webhook reaching a URL you set yourself.
- Denial of service that requires the `alertu` group or root to trigger.

## Reporting a vulnerability

**Please do not open a public issue for a security problem.**

Use GitHub's private reporting — the **Report a vulnerability** button under the
repository's **Security** tab
(<https://github.com/systm-d/alertU/security/advisories/new>). If you would
rather use email, write to **kevin@delfour.co**.

Please include the version (`alertu-ctl --version`), your distribution, and the
smallest reproduction you can manage. A proof of concept helps, but a clear
description of the boundary crossed is enough to start.

This is a personal project maintained in spare time, so there is no paid bounty
and no formal service-level agreement. What there is: an acknowledgement within
a week, an honest assessment of whether it is in scope, and — for anything that
is — a fix on the latest release and credit in the advisory, unless you would
rather stay anonymous.
