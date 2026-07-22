+++
title = "AlertU"

[extra]
eyebrow = "Linux · logind · evdev"
tagline = "Lock your machine. Stay in control."
lede = "A cheap USB or Bluetooth remote becomes the key fob for your Linux desktop. Click to arm: the session locks and a chirp plays. Touch the machine while it's armed and a countdown starts — then a siren, a timestamped webcam still, and an optional webhook."
cta = "View on GitHub"
cta2 = "Install"
logo_alt = "The AlertU logo: a neon shield in a green-to-cyan gradient, a red alarm bell inside it, red signal waves rising above."
langs_label = "Language"
term_toggle = "exactly what a remote click does"
+++

<section>
<p class="eyebrow">How it works</p>

## Four states, one key fob

<p class="section__lede">One task in the daemon owns all mutable state and drives every transition from four multiplexed sources: input signals, session lock changes, IPC commands, and internal timers.</p>

<ol class="flow">
  <li class="s1"><p class="st"><span class="dot"></span>Idle</p><p>Disarmed. Nothing is watched.</p></li>
  <li class="s2"><p class="st"><span class="dot"></span>Armed</p><p>A click on the remote locks the session with <code>loginctl lock-session</code> and plays a short chirp. Watched inputs go live after a grace period.</p></li>
  <li class="s3"><p class="st"><span class="dot"></span>Triggered</p><p>Activity on a watched device starts an adjustable countdown, with a discreet warning tick.</p></li>
  <li class="s4"><p class="st"><span class="dot"></span>Alarm</p><p>The countdown expired. The siren loops, a timestamped webcam still is saved, and the optional webhook fires.</p></li>
</ol>

<p class="flow-note"><strong>Disarming: whichever comes first wins.</strong> Another click on the remote, or a normal password unlock — read from logind's <code>LockedHint</code> over D-Bus in real time, falling back to polling <code>loginctl</code> when the bus is unavailable. Unlock during the countdown and everything resets to Idle.</p>
</section>

<hr class="divider" />
<section>
<p class="eyebrow">What it is</p>

## Small, local, and honest about it

<div class="feats">
  <div class="feat"><h3><span class="mark">◆</span> Any remote, any key</h3><p>Nothing is hardcoded to a model. Any USB or Bluetooth device that appears as a HID node under <code>/dev/input/eventX</code> works — a presentation clicker, a cheap Bluetooth shutter, a spare keyboard — and <code>toggle_keys</code> accepts any evdev key name.</p></div>
  <div class="feat"><h3><span class="mark">◆</span> X11 and Wayland</h3><p>It only talks to logind, so there is no dependency on a compositor or a desktop environment. Linux with systemd is the whole requirement.</p></div>
  <div class="feat"><h3><span class="mark">◆</span> Four binaries</h3><p>A privileged daemon, a StatusNotifierItem tray, a standalone egui settings window, and <code>alertu-ctl</code>. Every front end talks to the daemon over one local Unix socket, newline-delimited JSON.</p></div>
  <div class="feat"><h3><span class="mark">◆</span> Entirely local</h3><p>No cloud, no telemetry, no account. The only thing that ever leaves the machine is the webhook you configure yourself — and it is empty by default.</p></div>
  <div class="feat"><h3><span class="mark">◆</span> Pure Rust where it counts</h3><p>The tray uses <code>zbus</code>, not <code>libdbus</code>; no GTK, no Qt, no ALSA linkage. Audio, webcam capture and the webhook shell out to <code>paplay</code>/<code>ffplay</code>, <code>fswebcam</code>/<code>ffmpeg</code> and <code>curl</code>.</p></div>
  <div class="feat"><h3><span class="mark">◆</span> Tested and MIT</h3><p>102 tests — unit tests plus integration tests that drive a real daemon over its socket with a fake <code>loginctl</code> — run on every push, alongside <code>rustfmt</code> and clippy with <code>-D warnings</code>.</p></div>
</div>
</section>

<hr class="divider" />
<section>
<p class="eyebrow">Your remote</p>

## There is no default remote — deliberately

<p class="section__lede"><code>remote_name_hint</code> is empty out of the box, and empty means <em>no remote</em>, not "pick the first one". An empty substring would match every device, so AlertU resolves nothing rather than silently binding your toggle to whichever node enumerated first. Until you name a device, the daemon says so in its log and the remote toggle is simply unavailable — everything else still works.</p>

<div class="term">
<div class="term__bar" aria-hidden="true"><span class="term__dot"></span><span class="term__dot"></span><span class="term__dot"></span><span class="term__title">finding your remote</span></div>
<pre><code><span class="p">$</span> alertu-ctl list-devices
/dev/input/event3  AT Translated Set 2 keyboard [keyboard]
/dev/input/event5  Logitech USB Receiver [pointer]
/dev/input/event9  BT Camera Shutter [keyboard]

<span class="p">$</span> sudo journalctl -u alertu-daemon -f   <span class="dim"># press a button, at RUST_LOG=debug</span></code></pre>
</div>

<p class="flow-note">Then set <code>remote_name_hint = "shutter"</code> and <code>toggle_keys = ["KEY_VOLUMEUP"]</code> — from the tray, from the settings window, or with <code>alertu-ctl set-config</code>. Watched devices default to <code>["auto"]</code>: everything except the remote and the main mouse.</p>
</section>

<hr class="divider" />
<section>
<p class="eyebrow">Command line</p>

## Everything the tray does, from a script

<ul class="commands">
  <li><code>alertu-ctl status</code><span>Idle, Armed, Triggered or Alarm</span></li>
  <li><code>alertu-ctl status --watch</code><span>one line per state change, until interrupted</span></li>
  <li><code>alertu-ctl arm</code><span>force-arm — locks the session</span></li>
  <li><code>alertu-ctl disarm</code><span>force-disarm — unlocks it</span></li>
  <li><code>alertu-ctl toggle</code><span>exactly what a remote click does</span></li>
  <li><code>alertu-ctl get-config</code><span>the daemon's effective config, as TOML</span></li>
  <li><code>alertu-ctl set-config c.toml</code><span>replace it (<code>-</code> reads stdin), validated locally first</span></li>
  <li><code>alertu-ctl list-devices</code><span>the input devices the daemon can see</span></li>
  <li><code>alertu-ctl gen-sounds --dir …</code><span>write the three default sound files</span></li>
</ul>

<p class="flow-note"><code>--json</code> prints the daemon's raw protocol response, so a watched transition arrives as <code>{"event":"state_changed","state":"armed"}</code> and stays distinguishable from the initial <code>state</code> snapshot. Exit codes: <code>0</code> success, <code>1</code> daemon or connection error, <code>2</code> usage error.</p>
</section>

<hr class="divider" />
<section>
<p class="eyebrow">Scope</p>

## A personal gadget, not an anti-theft system

<div class="callout callout--alarm">
<p>That is the project's own wording, and the site will not pretend otherwise. There is no binary anti-tampering. The control socket is <code>0660</code> in the daemon's group, and connecting to it is equivalent to full control of the alarm — disarming it, reading the config including the webhook URL, and <code>SetConfig</code>, which steers the paths handed to the helper programs. Treat group membership as a privilege grant, not a convenience.</p>
<p>Alarm snapshots sit behind the same boundary: each still is written <code>0640</code> in that group, inside a <code>snapshot_dir</code> the daemon keeps at <code>0750</code> when it owns it. Deliberately not world-readable — a webcam photograph of whoever is at the machine, the owner included, has no business being. A directory the daemon does not own is left exactly as it is, with a warning, rather than re-permissioned.</p>
</div>
</section>

<hr class="divider" />
<section id="install">
<p class="eyebrow">Install</p>

## Built from source, today

<p class="section__lede">There is no released package yet — a Debian package is in progress. For now, build it and install the bundled systemd units. The full walkthrough, including the tray's user unit, the icons and the desktop entry, is in the <a href="https://github.com/systm-d/alertU#install-systemd">README</a>.</p>

<div class="steps">
  <div class="step">
    <h3>1 · Build</h3>

```sh
git clone https://github.com/systm-d/alertU
cd alertU
cargo build --release
```

<p>Needs a recent stable Rust toolchain. Only <code>alertu-settings</code> pulls in system build dependencies (egui links X11/Wayland/GL) — skip it with <code>--workspace --exclude alertu-settings</code>.</p>
  </div>
  <div class="step">
    <h3>2 · Service account &amp; units</h3>

```sh
sudo systemd-sysusers packaging/sysusers.d/alertu.conf
sudo install -Dm644 packaging/alertu-daemon.service \
  /etc/systemd/system/alertu-daemon.service
sudo alertu-ctl gen-sounds --dir /usr/share/sounds/alertu
sudo systemctl enable --now alertu-daemon
```

<p>The daemon's own account is the one that needs <code>input</code> and <code>video</code>; yours needs neither.</p>
  </div>
  <div class="step">
    <h3>3 · Join the socket's group</h3>

```sh
sudo usermod -aG alertu "$USER"
# then start a new session, or: newgrp alertu
```

<p>The socket is <code>0660</code>. Without that group the tray, the settings window and <code>alertu-ctl</code> all fail to connect.</p>
  </div>
  <div class="step">
    <h3>4 · External helpers</h3>

```sh
# snapshots: one of
fswebcam  |  ffmpeg
# audio: one of
paplay  |  pw-play  |  aplay  |  ffplay  |  play
```

<p>AlertU shells out to these rather than linking them, so pick whichever your distribution already ships.</p>
  </div>
</div>

<p class="callout"><strong>A Linux re-imagining of the old Mac iAlertU.</strong> MIT licensed, built in the open at <a href="https://github.com/systm-d/alertU">systm-d/alertU</a>. Configuration lives in <code>/etc/alertu/config.toml</code>, and every field is documented inline in <a href="https://github.com/systm-d/alertU/blob/main/packaging/config.example.toml"><code>packaging/config.example.toml</code></a>.</p>
</section>
