//! AlertU settings window.
//!
//! A small egui/eframe app that talks to the daemon over its Unix socket to
//! load the current config and device list, lets the user edit everything
//! (remote, watched devices, delays, sounds, camera, webhook), and pushes the
//! result back with `SetConfig`. Launched standalone or from the tray's
//! "Open settings…" item.

mod client;

use alertu_common::config::{Config, AUTO};
use alertu_common::protocol::{InputDeviceInfo, DEFAULT_SOCKET_PATH};
use alertu_common::state::GuardState;
use client::Client;
use eframe::egui;
use std::path::PathBuf;

fn socket_path() -> PathBuf {
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        if matches!(arg.as_str(), "--socket" | "-s") {
            if let Some(v) = it.next() {
                return PathBuf::from(v);
            }
        }
    }
    PathBuf::from(DEFAULT_SOCKET_PATH)
}

fn main() -> eframe::Result<()> {
    let socket = socket_path();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([540.0, 680.0])
            .with_min_inner_size([420.0, 400.0])
            .with_title("AlertU — Settings"),
        ..Default::default()
    };
    eframe::run_native(
        "AlertU Settings",
        options,
        Box::new(move |_cc| Ok(Box::new(SettingsApp::new(socket)))),
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
        };
        app.reconnect();
        app
    }

    /// (Re)connect and load config + devices + state from the daemon.
    fn reconnect(&mut self) {
        match Client::connect(&self.socket) {
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
            }
            Err(e) => {
                self.client = None;
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
    fn save(&mut self) {
        let cfg = self.collect();
        if let Err(e) = cfg.validate() {
            self.status = format!("Invalid: {e}");
            return;
        }
        let Some(c) = self.client.as_mut() else {
            self.status = "Not connected.".into();
            return;
        };
        match c.set_config(cfg.clone()) {
            Ok(()) => {
                self.cfg = cfg;
                self.status = "Saved and applied.".into();
            }
            Err(e) => self.status = format!("Save failed: {e}"),
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
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("AlertU");
                ui.separator();
                match self.state {
                    Some(s) => ui.label(format!("Daemon state: {s}")),
                    None => ui.colored_label(egui::Color32::YELLOW, "daemon not connected"),
                };
            });
        });

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
                self.remote_section(ui);
                ui.add_space(8.0);
                self.watch_section(ui);
                ui.add_space(8.0);
                self.timing_section(ui);
                ui.add_space(8.0);
                self.media_section(ui);
            });
        });
    }
}

impl SettingsApp {
    fn remote_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Remote")
            .default_open(true)
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
                }

                ui.label("Toggle keys (comma-separated evdev names):");
                ui.text_edit_singleline(&mut self.toggle_keys_buf);
                ui.small("e.g. KEY_VOLUMEUP, KEY_ENTER");
            });
    }

    fn watch_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Watched devices (intrusion detection)")
            .default_open(true)
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

    fn timing_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Timing")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Grace period before detection:");
                    ui.add(
                        egui::DragValue::new(&mut self.cfg.grace_period_secs)
                            .range(0..=3600)
                            .suffix(" s"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Alarm delay (Triggered → siren):");
                    ui.add(
                        egui::DragValue::new(&mut self.cfg.alarm_delay_secs)
                            .range(1..=3600)
                            .suffix(" s"),
                    );
                });
            });
    }

    fn media_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Sounds, camera & hook")
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
