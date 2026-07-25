//! AlertU settings window.
//!
//! A small egui/eframe app that talks to the daemon over its Unix socket to load
//! the current config and device list, and pushes changes back with `SetConfig`.
//! Launched standalone or from the tray's "Open settings…" item.
//!
//! The layout answers a complaint that the previous one was too complicated: the
//! three things a working installation actually needs — a paired remote, an
//! audible output, two delays — are on screen, and everything else sits behind
//! "Advanced settings", collapsed. Nothing was removed; the manual device and
//! key fields are still there for anyone who wants them, one click away.

#![forbid(unsafe_code)]

mod theme;

use alertu_common::config::{AUTO, Config};
use alertu_common::ipc_client::{Client, LearnedRemote};
use alertu_common::protocol::{DEFAULT_SOCKET_PATH, InputDeviceInfo};
use alertu_common::state::GuardState;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

/// How long the daemon listens for a button press during pairing.
const PAIRING_WINDOW_SECS: u64 = 30;

/// Helper that adds the calling account to the daemon's group, run via `pkexec`.
/// Pinned by the polkit action in packaging/polkit/dev.systm-d.alertu.policy.
const AUTHORIZE_HELPER: &str = "/usr/lib/alertu/alertu-authorize";

/// A blocking call moved off the drawing thread.
///
/// Both things this window has to wait for — a button press, a password prompt —
/// take seconds to tens of seconds. Run inline they would freeze the window, and
/// a frozen egui window is what a desktop reports as "not responding".
struct Job<T> {
    rx: Receiver<T>,
}

impl<T> Job<T> {
    /// Spawn `work` on its own thread.
    fn spawn(work: impl FnOnce() -> T + Send + 'static) -> Self
    where
        T: Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        Job { rx }
    }
}

fn socket_path() -> PathBuf {
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        if matches!(arg.as_str(), "--socket" | "-s")
            && let Some(v) = it.next()
        {
            return PathBuf::from(v);
        }
    }
    PathBuf::from(DEFAULT_SOCKET_PATH)
}

/// Whether the socket exists and actively refused this account.
///
/// `EACCES` is not "the daemon is down": the socket is `0660` and group-owned, so
/// a permission error means the daemon is up and this login is simply not in its
/// group — the one failure the window can offer to fix.
fn socket_forbidden(socket: &std::path::Path) -> bool {
    match std::os::unix::net::UnixStream::connect(socket) {
        Err(e) => e.kind() == std::io::ErrorKind::PermissionDenied,
        Ok(_) => false,
    }
}

/// ALSA output devices, as `aplay -L` reports them.
///
/// Only the `plughw:` entries are kept. `default` and `sysdefault` route through
/// the session's sound server, which is precisely what the daemon cannot reach —
/// offering them would be offering the failure this setting exists to avoid. Bare
/// `hw:` is dropped too: it refuses the format conversion the wav files need.
///
/// Names come back in `plughw:CARD=…,DEV=…` form rather than `plughw:0,2`, which
/// survives cards being renumbered — the same reasoning that makes the remote
/// match by name.
fn alsa_outputs() -> Vec<(String, String)> {
    let output = match std::process::Command::new("aplay").arg("-L").output() {
        Ok(o) if o.status.success() => o.stdout,
        // No aplay, or it failed: the field stays free-text.
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&output);

    let mut devices: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.starts_with(char::is_whitespace) {
            // Continuation: the first indented line describes the device above.
            if let Some((_, description)) = devices.last_mut()
                && description.is_empty()
            {
                *description = line.trim().to_string();
            }
        } else if line.starts_with("plughw:") {
            devices.push((line.trim().to_string(), String::new()));
        }
    }
    devices
}

fn main() -> eframe::Result<()> {
    let socket = socket_path();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 620.0])
            .with_min_inner_size([440.0, 380.0])
            .with_title("AlertU — Settings"),
        ..Default::default()
    };
    eframe::run_native(
        "AlertU Settings",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(SettingsApp::new(socket)))
        }),
    )
}

/// The window state: the daemon connection, the working copy of the config, and
/// string buffers for the free-text/path fields egui edits in place.
struct SettingsApp {
    socket: PathBuf,
    client: Option<Client>,
    state: Option<GuardState>,
    cfg: Config,
    devices: Vec<InputDeviceInfo>,
    status: String,

    // String buffers for PathBuf fields and the comma-separated key list.
    beep_buf: String,
    warning_buf: String,
    siren_buf: String,
    snapshot_buf: String,
    toggle_keys_buf: String,

    /// Set while waiting for the user to press a button.
    pairing: Option<Job<Result<Option<LearnedRemote>, String>>>,
    /// Set while the polkit password prompt is up.
    authorizing: Option<Job<Result<String, String>>>,
    /// The socket exists but refused this account: it is not in the daemon's
    /// group. Cached, since answering it means an extra connect() and this is
    /// consulted every frame.
    forbidden: bool,
    /// ALSA outputs offered in the picker. Cached: filling it spawns `aplay -L`,
    /// which has no business running once per frame.
    alsa_outputs: Vec<(String, String)>,
}

impl SettingsApp {
    fn new(socket: PathBuf) -> Self {
        let mut app = SettingsApp {
            socket,
            client: None,
            state: None,
            cfg: Config::default(),
            devices: Vec::new(),
            status: String::new(),
            beep_buf: String::new(),
            warning_buf: String::new(),
            siren_buf: String::new(),
            snapshot_buf: String::new(),
            toggle_keys_buf: String::new(),
            pairing: None,
            authorizing: None,
            forbidden: false,
            alsa_outputs: alsa_outputs(),
        };
        app.reconnect();
        app
    }

    /// Ask the daemon to play its beep, saving first so it plays through the
    /// output currently shown rather than the last saved one.
    fn test_sound(&mut self) {
        if !self.save() {
            return;
        }
        let Some(c) = self.client.as_mut() else {
            self.status = "Not connected.".into();
            return;
        };
        match c.test_sound() {
            // The daemon acknowledges queueing the sound, not producing it — the
            // player is a separate process that can still fail. So this claims
            // only what it knows, and points at where the reason would be.
            Ok(()) => {
                self.status = "Beep sent. Silence means the output is wrong — \
                               `journalctl -u alertu-daemon` says why."
                    .into()
            }
            Err(e) => self.status = format!("Test failed: {e}"),
        }
    }

    /// Ask the daemon to watch for a button press, on a background thread.
    ///
    /// The reply is withheld until a key arrives — up to `PAIRING_WINDOW_SECS` —
    /// and this app is synchronous, so doing it inline would freeze the window
    /// for the whole window. The thread opens its own connection so the main one
    /// stays free to answer the rest of the UI.
    fn start_pairing(&mut self) {
        let socket = self.socket.clone();
        self.pairing = Some(Job::spawn(move || {
            Client::connect(&socket)
                .and_then(|mut client| client.learn_remote(PAIRING_WINDOW_SECS))
                .map_err(|e| e.to_string())
        }));
        self.status = "Press a button on your remote…".into();
    }

    /// Add this account to the daemon's group through `pkexec`.
    ///
    /// The helper takes no arguments and derives the account from `PKEXEC_UID`,
    /// so this can only grant the group to the person who clicked.
    fn start_authorizing(&mut self) {
        self.authorizing = Some(Job::spawn(|| {
            match std::process::Command::new("pkexec")
                .arg(AUTHORIZE_HELPER)
                .status()
            {
                Ok(status) if status.success() => Ok(
                    "Account authorized. Log out and back in — group membership is \
                     only picked up by a new session."
                        .to_string(),
                ),
                // pkexec's own exit codes: 126 is a dismissed dialog or refused
                // authorization, 127 is a helper it could not run.
                Ok(status) if status.code() == Some(126) => {
                    Err("Authorization cancelled.".to_string())
                }
                Ok(status) if status.code() == Some(127) => Err(format!(
                    "pkexec could not run {AUTHORIZE_HELPER} — is the package installed?"
                )),
                Ok(status) => Err(format!("Authorization failed (exit {:?}).", status.code())),
                Err(e) => Err(format!("Could not run pkexec: {e}")),
            }
        }));
        self.status = "Waiting for the authorization prompt…".into();
    }

    /// Poll the authorization helper.
    fn poll_authorizing(&mut self, ctx: &egui::Context) {
        let Some(job) = self.authorizing.as_ref() else {
            return;
        };
        match job.rx.try_recv() {
            Ok(Ok(message)) => {
                self.authorizing = None;
                self.status = message;
                // Deliberately no reconnect(): this process still carries the
                // groups it was started with, so a retry would fail and read as
                // the authorization not having worked.
            }
            Ok(Err(message)) => {
                self.authorizing = None;
                self.status = message;
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
            Err(TryRecvError::Disconnected) => {
                self.authorizing = None;
                self.status = "Authorization stopped unexpectedly.".into();
            }
        }
    }

    /// Poll a running pairing attempt, saving a learned remote as it lands.
    fn poll_pairing(&mut self, ctx: &egui::Context) {
        let Some(pairing) = self.pairing.as_ref() else {
            return;
        };
        match pairing.rx.try_recv() {
            Ok(Ok(Some(learned))) => {
                self.pairing = None;
                self.cfg
                    .apply_learned_remote(&learned.path, &learned.name, &learned.key);
                // The key list is edited through a buffer, so it has to be
                // refreshed before `save` collects it — otherwise the stale
                // buffer would overwrite the key just learned.
                self.load_buffers();
                if self.save() {
                    self.status = format!("Paired {} ({}).", learned.name, learned.key);
                }
            }
            Ok(Ok(None)) => {
                self.pairing = None;
                self.status = "No button detected. The remote may be asleep — \
                               press a button to wake it, then try again."
                    .into();
            }
            Ok(Err(e)) => {
                self.pairing = None;
                self.status = format!("Pairing failed: {e}");
            }
            Err(TryRecvError::Empty) => {
                // egui only redraws on input, and the user is pressing a button
                // on a device this window never sees, so ask for a wake-up or
                // the result would sit unread until the mouse moves.
                ctx.request_repaint_after(Duration::from_millis(200));
            }
            Err(TryRecvError::Disconnected) => {
                self.pairing = None;
                self.status = "Pairing stopped unexpectedly.".into();
            }
        }
    }

    /// (Re)connect and load config + devices + state from the daemon.
    fn reconnect(&mut self) {
        match Client::connect(&self.socket).map(Client::with_reconnect) {
            Ok(mut c) => {
                match (c.get_config(), c.list_devices(), c.get_state()) {
                    (Ok(cfg), Ok(devices), state) => {
                        self.cfg = cfg;
                        self.devices = devices;
                        self.state = state.ok();
                        self.load_buffers();
                        self.status = "Loaded config from daemon.".into();
                    }
                    (Err(e), _, _) | (_, Err(e), _) => {
                        self.status = format!("Load failed: {e}");
                    }
                }
                self.client = Some(c);
                self.forbidden = false;
            }
            Err(e) => {
                self.client = None;
                self.forbidden = socket_forbidden(&self.socket);
                self.status = format!("Not connected: {e}");
            }
        }
    }

    /// Refresh just the device list (e.g. after a hotplug).
    fn refresh_devices(&mut self) {
        if let Some(c) = self.client.as_mut() {
            match c.list_devices() {
                Ok(d) => {
                    self.devices = d;
                    self.status = format!("{} device(s) found.", self.devices.len());
                }
                Err(e) => self.status = format!("Refresh failed: {e}"),
            }
        } else {
            self.reconnect();
        }
    }

    /// Fill the string buffers from the current config.
    fn load_buffers(&mut self) {
        self.beep_buf = self.cfg.beep_sound.display().to_string();
        self.warning_buf = self.cfg.warning_sound.display().to_string();
        self.siren_buf = self.cfg.siren_sound.display().to_string();
        self.snapshot_buf = self.cfg.snapshot_dir.display().to_string();
        self.toggle_keys_buf = self.cfg.toggle_keys.join(", ");
    }

    /// Assemble a Config from the working copy + buffers.
    fn collect(&self) -> Config {
        let mut cfg = self.cfg.clone();
        cfg.beep_sound = PathBuf::from(self.beep_buf.trim());
        cfg.warning_sound = PathBuf::from(self.warning_buf.trim());
        cfg.siren_sound = PathBuf::from(self.siren_buf.trim());
        cfg.snapshot_dir = PathBuf::from(self.snapshot_buf.trim());
        cfg.toggle_keys = self
            .toggle_keys_buf
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        cfg
    }

    /// Validate and push the config to the daemon.
    ///
    /// Returns whether it was saved, so a caller with something better to report
    /// can tell success from failure without matching on `status` text.
    fn save(&mut self) -> bool {
        let cfg = self.collect();
        if let Err(e) = cfg.validate() {
            self.status = format!("Invalid: {e}");
            return false;
        }
        let Some(c) = self.client.as_mut() else {
            self.status = "Not connected.".into();
            return false;
        };
        match c.set_config(cfg.clone()) {
            Ok(()) => {
                self.cfg = cfg;
                self.status = "Saved and applied.".into();
                true
            }
            Err(e) => {
                self.status = format!("Save failed: {e}");
                false
            }
        }
    }

    /// Path currently set as the remote (None when "auto").
    fn remote_path(&self) -> Option<String> {
        (!self.cfg.remote_is_auto()).then(|| self.cfg.remote_device.clone())
    }

    /// The explicit watch list implied by the current devices (non-pointer, not
    /// the remote) — used when switching off "auto".
    fn effective_watch_explicit(&self) -> Vec<String> {
        let remote = self.remote_path();
        self.devices
            .iter()
            .filter(|d| !d.is_pointer && Some(d.path.to_string_lossy().to_string()) != remote)
            .map(|d| d.path.to_string_lossy().to_string())
            .collect()
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_pairing(ctx);
        self.poll_authorizing(ctx);

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("AlertU");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The site already has a colour per alarm state; reusing them
                    // means the badge needs no invention of its own.
                    match self.state {
                        Some(s) => ui.colored_label(
                            theme::state_color(s),
                            format!("● {}", theme::state_label(s)),
                        ),
                        None => ui.colored_label(theme::MUT, "● daemon not connected"),
                    };
                });
            });
            ui.add_space(4.0);
        });

        // The one obstacle a fresh install always hits: the socket is 0660 and
        // group-owned, and no package may add an existing account to a group.
        // Offer the grant here instead of sending the user to a terminal.
        if self.forbidden {
            egui::TopBottomPanel::top("authorize").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(theme::ALARM, "This account may not control AlertU.");
                    let busy = self.authorizing.is_some();
                    ui.add_enabled_ui(!busy, |ui| {
                        if ui
                            .button(if busy {
                                "Waiting for the prompt…"
                            } else {
                                "Authorize this account"
                            })
                            .clicked()
                        {
                            self.start_authorizing();
                        }
                    });
                });
            });
        }

        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Reload").clicked() {
                    self.reconnect();
                }
                if ui.button("Refresh devices").clicked() {
                    self.refresh_devices();
                }
                let connected = self.client.is_some();
                ui.add_enabled_ui(connected, |ui| {
                    if ui.button("Save").clicked() {
                        self.save();
                    }
                });
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.pairing_card(ui);
                ui.add_space(10.0);
                self.sound_card(ui);
                ui.add_space(10.0);
                self.delays_card(ui);
                ui.add_space(14.0);
                self.advanced(ui);
            });
        });
    }
}

/// A titled panel, the window's unit of layout.
fn card(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            ui.strong(title);
            ui.add_space(4.0);
            contents(ui);
        });
    });
}

impl SettingsApp {
    /// The remote card: one button, and what it found.
    ///
    /// Deliberately the whole of the remote for most people. The manual fields
    /// live under "Advanced settings" because using them means knowing which
    /// `/dev/input` node the remote is *and* the evdev name of the key it sends —
    /// and a key the remote cannot emit fails silently, producing a remote that
    /// looks configured and never arms.
    fn pairing_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Remote", |ui| {
            let paired = !self.cfg.toggle_keys.is_empty()
                && (!self.cfg.remote_name_hint.trim().is_empty() || !self.cfg.remote_is_auto());
            if paired {
                let which = if self.cfg.remote_is_auto() {
                    self.cfg.remote_name_hint.clone()
                } else {
                    self.cfg.remote_device.clone()
                };
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(theme::SAFE, "●");
                    ui.label(format!("{which} — {}", self.cfg.toggle_keys.join(", ")));
                });
            } else {
                ui.colored_label(theme::MUT, "No remote paired yet.");
            }
            ui.add_space(6.0);

            let pairing = self.pairing.is_some();
            ui.add_enabled_ui(self.client.is_some() && !pairing, |ui| {
                if ui
                    .button(if pairing {
                        "Waiting for a button press…"
                    } else if paired {
                        "Pair a different remote"
                    } else {
                        "Pair remote"
                    })
                    .clicked()
                {
                    self.start_pairing();
                }
            });
            if pairing {
                ui.colored_label(
                    theme::BLUE_INK,
                    format!("Press a button on your remote within {PAIRING_WINDOW_SECS}s."),
                );
            } else {
                ui.small("Press its button — AlertU finds the device and the key itself.");
            }
        });
    }

    /// The audio card: where the siren comes out, and proof that it does.
    fn sound_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Alarm sound", |ui| {
            ui.horizontal(|ui| {
                ui.label("Output:");
                let selected = if self.cfg.alsa_device.trim().is_empty() {
                    "Automatic (often silent for a service)".to_string()
                } else {
                    self.cfg.alsa_device.clone()
                };
                egui::ComboBox::from_id_salt("alsa_device")
                    .selected_text(selected)
                    .width(300.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.cfg.alsa_device,
                            String::new(),
                            "Automatic (often silent for a service)",
                        );
                        for (name, description) in &self.alsa_outputs {
                            let label = if description.is_empty() {
                                name.clone()
                            } else {
                                format!("{description}  —  {name}")
                            };
                            ui.selectable_value(&mut self.cfg.alsa_device, name.clone(), label);
                        }
                    });
                ui.add_enabled_ui(self.client.is_some(), |ui| {
                    if ui.button("Test").clicked() {
                        self.test_sound();
                    }
                });
            });
            // Naming the trap plainly: the daemon has no session, so the default
            // output — a session sound server — is the one thing it cannot use.
            ui.small(
                "The daemon runs without a desktop session, so it cannot reach PipeWire \
                 or PulseAudio. Pick your built-in speaker: a siren in headphones on the \
                 desk protects nothing.",
            );
            if self.alsa_outputs.is_empty() {
                ui.colored_label(
                    theme::MUT,
                    "No ALSA outputs listed — install alsa-utils for `aplay -L`.",
                );
            }
        });
    }

    /// The two delays worth tuning.
    fn delays_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Delays", |ui| {
            egui::Grid::new("delays_grid")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Grace before detection:");
                    ui.add(
                        egui::DragValue::new(&mut self.cfg.grace_period_secs)
                            .range(0..=3600)
                            .suffix(" s"),
                    );
                    ui.end_row();
                    ui.label("Siren after intrusion:");
                    ui.add(
                        egui::DragValue::new(&mut self.cfg.alarm_delay_secs)
                            .range(1..=3600)
                            .suffix(" s"),
                    );
                    ui.end_row();
                });
            ui.small("Grace lets you walk away after arming without setting it off yourself.");
        });
    }

    /// Everything else, collapsed. Nothing here was removed from the window — it
    /// was moved out of the way of the three settings that decide whether AlertU
    /// works at all.
    fn advanced(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Advanced settings")
            .default_open(false)
            .show(ui, |ui| {
                self.manual_remote_section(ui);
                ui.add_space(8.0);
                self.watch_section(ui);
                ui.add_space(8.0);
                self.media_section(ui);
            });
    }

    fn manual_remote_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Remote, by hand")
            .default_open(false)
            .show(ui, |ui| {
                let selected_text = if self.cfg.remote_is_auto() {
                    "Auto (by name hint)".to_string()
                } else {
                    self.cfg.remote_device.clone()
                };
                egui::ComboBox::from_label("Remote device")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.cfg.remote_device,
                            AUTO.to_string(),
                            "Auto (by name hint)",
                        );
                        for dev in &self.devices {
                            let path = dev.path.to_string_lossy().to_string();
                            let label = format!(
                                "{}{} — {}",
                                if dev.is_pointer { "[pointer] " } else { "" },
                                dev.name,
                                path
                            );
                            ui.selectable_value(&mut self.cfg.remote_device, path, label);
                        }
                    });

                if self.cfg.remote_is_auto() {
                    ui.horizontal(|ui| {
                        ui.label("Name hint:");
                        ui.text_edit_singleline(&mut self.cfg.remote_name_hint);
                    });
                    ui.small("Matched against device names; survives a Bluetooth reconnect.");
                }

                ui.label("Toggle keys (comma-separated evdev names):");
                ui.text_edit_singleline(&mut self.toggle_keys_buf);
                ui.small(
                    "e.g. KEY_VOLUMEUP. A key the remote cannot emit never matches — \
                     the daemon warns about that in its log.",
                );
            });
    }

    fn watch_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Watched devices (intrusion detection)")
            .default_open(false)
            .show(ui, |ui| {
                let mut auto = self.cfg.watch_is_auto();
                if ui
                    .checkbox(
                        &mut auto,
                        "Auto — all devices except the remote and main mouse",
                    )
                    .changed()
                {
                    self.cfg.watch_devices = if auto {
                        vec![AUTO.to_string()]
                    } else {
                        self.effective_watch_explicit()
                    };
                }

                let remote = self.remote_path();
                ui.add_enabled_ui(!auto, |ui| {
                    for dev in &self.devices {
                        let path = dev.path.to_string_lossy().to_string();
                        if Some(&path) == remote.as_ref() {
                            continue; // the remote is never watched
                        }
                        let mut checked = self.cfg.watch_devices.iter().any(|w| w == &path);
                        let label = format!(
                            "{}{} — {}",
                            if dev.is_pointer { "[pointer] " } else { "" },
                            dev.name,
                            path
                        );
                        if ui.checkbox(&mut checked, label).changed() {
                            if checked {
                                self.cfg.watch_devices.push(path);
                            } else {
                                self.cfg.watch_devices.retain(|w| w != &path);
                            }
                        }
                    }
                });

                if self.devices.is_empty() {
                    ui.small("No devices listed — click “Refresh devices”.");
                }
            });
    }

    fn media_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Sound files, camera & hook")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("media_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Beep sound:");
                        ui.text_edit_singleline(&mut self.beep_buf);
                        ui.end_row();
                        ui.label("Warning sound:");
                        ui.text_edit_singleline(&mut self.warning_buf);
                        ui.end_row();
                        ui.label("Siren sound:");
                        ui.text_edit_singleline(&mut self.siren_buf);
                        ui.end_row();
                        ui.label("Snapshot dir:");
                        ui.text_edit_singleline(&mut self.snapshot_buf);
                        ui.end_row();
                        ui.label("Camera device:");
                        ui.text_edit_singleline(&mut self.cfg.camera_device);
                        ui.end_row();
                        ui.label("Session id:");
                        ui.text_edit_singleline(&mut self.cfg.session_id);
                        ui.end_row();
                        ui.label("Alarm webhook URL:");
                        ui.text_edit_singleline(&mut self.cfg.alarm_webhook_url);
                        ui.end_row();
                    });
            });
    }
}
