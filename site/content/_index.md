+++
title = "AlertU"

[extra]
nav_label = "Primary"
nav_docs = "Docs"
nav_download = "Download"
langs_label = "Language"

hero_eyebrow = "Like a car alarm — for your computer"
hero_title = "Lock it. Walk away. AlertU stands guard."
hero_lede = "Arm your machine with a click: the screen locks, a chirp plays, and a silent watch begins. Touch it while you are gone and AlertU sounds off, snaps a photo, and pings your phone."
cta_download = "Download"
cta_github = "GitHub"
cta_docs = "Documentation"
hero_meta = "Free & open source · Linux + systemd · MIT"

how_eyebrow = "How it works"
how_title = "Six steps, one reflex"
how_lede = "The same loop a car alarm runs — arm, watch, react — living quietly in a daemon on your machine."
step_arm = "Arm"
step_arm_d = "One click locks the session."
step_leave = "Leave"
step_leave_d = "Grab a coffee. The watch goes live."
step_detect = "Detect"
step_detect_d = "A touched key or mouse trips it."
step_alarm = "Alarm"
step_alarm_d = "A countdown, then the siren."
step_snap = "Snapshot"
step_snap_d = "The webcam saves a still."
step_disarm = "Disarm"
step_disarm_d = "Click again — back to calm."

detect_eyebrow = "Intrusion"
detect_title = "It knows the moment someone touches it"
detect_body = "Once armed, AlertU watches your input devices at the kernel level. A nudge of the mouse, a tap on the keyboard, a brush of the trackpad — anything on a watched device starts an adjustable countdown, with a discreet warning tick."
detect_t1 = "Watches any input device — keyboard, mouse, trackpad"
detect_t2 = "A grace period after arming, so you can walk away clean"
detect_t3 = "X11 or Wayland — it only talks to logind"

alarm_eyebrow = "Alarm"
alarm_title = "Then it makes noise"
alarm_body = "When the countdown runs out, the siren loops and the machine stops being subtle. Wire the optional webhook to your phone, Slack or Discord and the alert lands in your pocket, wherever you are."
alarm_t1 = "A looping siren, loud enough to turn heads"
alarm_t2 = "An optional webhook — send it to your phone, your way"

capture_eyebrow = "Evidence"
capture_title = "And remembers the face"
capture_body = "The instant the alarm fires, AlertU saves a timestamped still from the webcam. Come back to a small gallery of exactly who leaned in while you were gone."
capture_t1 = "Timestamped stills, saved locally"
capture_t2 = "Kept private — mode 0640, never world-readable"

disarm_eyebrow = "Disarm"
disarm_title = "Stand down the way you armed it"
disarm_body = "Your remote is the key fob. A second click, or a normal password unlock, drops everything straight back to calm. Whichever comes first wins — unlock during the countdown and nothing ever fires."
disarm_t1 = "Any USB or Bluetooth remote becomes the fob"
disarm_t2 = "Or just unlock the screen — logind tells AlertU in real time"

dev_eyebrow = "Built for developers"
dev_title = "Small, local, and honest about it"
dev_lede = "No cloud, no account, no telemetry. Four tiny binaries talking over one local socket."
card1_t = "Any Linux desktop"
card1_d = "X11 and Wayland, any distro with systemd. No desktop environment required."
card2_t = "Open source"
card2_d = "MIT licensed, built in the open. Read every line, send a patch."
card3_t = "Privacy first"
card3_d = "Everything stays on your machine. The only thing that leaves is the webhook you set."
card4_t = "Fast setup"
card4_d = "One package for your distro, name your remote, and you are armed in a minute."
card5_t = "Lightweight"
card5_d = "Pure Rust where it counts. No GTK, no Qt, no ALSA linkage."
card6_t = "Tested to the core"
card6_d = "102 tests on every push, with rustfmt and clippy at -D warnings."

scen_eyebrow = "In practice"
scen_title = "A coffee break, guarded"
scen_lede = "The whole story, the way it actually plays out."
scen1 = "You kick off a long compile."
scen2 = "You arm AlertU and stand up."
scen3 = "Someone touches your keyboard."
scen4 = "The siren goes off."
scen5 = "A snapshot is captured."
scen6 = "You click your remote — calm again."

install_eyebrow = "Install"
install_title = "A package for your distribution"
install_lede = "Every release ships an .rpm and a .deb — all four binaries, the systemd units and the sounds. On any other distro, build it with cargo."
install_source = "Source"
install_note = 'A package cannot add you to a group or start a user unit, so a couple of lines are yours. Full walkthrough in the <a href="https://github.com/systm-d/alertU#readme">README</a>.'

footer_quote_1 = "Always watching."
footer_quote_2 = "Until you come back."

ico_arm = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/></svg>'
ico_leave = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M13 3h6a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1h-6"/><path d="M10 12H3"/><path d="M6 8l-4 4 4 4"/></svg>'
ico_detect = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/></svg>'
ico_alarm = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M18 9a6 6 0 0 0-12 0c0 6-3 8-3 8h18s-3-2-3-8"/><path d="M10 20a2 2 0 0 0 4 0"/></svg>'
ico_snap = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 8h3l1.5-2h7L18 8h2a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1Z"/><circle cx="12" cy="13" r="3.5"/></svg>'
ico_disarm = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M8.5 12.5l2.5 2.5 4.5-5.5"/></svg>'
ico_touch = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M5 3l13 8-5.5 1.2L11 18z"/></svg>'
ico_coffee = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M5 9h11v5a4 4 0 0 1-4 4H9a4 4 0 0 1-4-4z"/><path d="M16 10h2a2 2 0 0 1 0 4h-2"/><path d="M8 3v2M11.5 3v2"/></svg>'
ico_platform = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/></svg>'
ico_open = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M8 6l-5 6 5 6"/><path d="M16 6l5 6-5 6"/></svg>'
ico_privacy = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l7 3v6c0 4-3 6.5-7 8-4-1.5-7-4-7-8V6z"/><path d="M9.5 12l2 2 3.5-4"/></svg>'
ico_fast = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M13 2 4 14h6l-1 8 9-12h-6z"/></svg>'
ico_light = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 20c0-8 6-14 16-15C19 13 13 20 4 20Z"/><path d="M9 15c2-2 5-3 8-4"/></svg>'
ico_secure = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/><circle cx="12" cy="15.5" r="1.2"/></svg>'
+++
