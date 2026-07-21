//! Turning daemon replies into terminal output.
//!
//! Rendering is kept separate from the socket round trip, and pure, so the
//! human and `--json` forms can be asserted directly in unit tests without a
//! running daemon.

use alertu_common::config::Config;
use alertu_common::protocol::{InputDeviceInfo, Response};
use alertu_common::state::GuardState;
use anyhow::{Context, Result};

/// What a command produced, before rendering.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    State(GuardState),
    Config(Box<Config>),
    Devices(Vec<InputDeviceInfo>),
    /// A command that only needed acknowledgement (arm/disarm/toggle/set-config).
    Ack,
}

impl Outcome {
    /// The protocol response this outcome came from, so `--json` re-emits the
    /// daemon's own wire shapes rather than a second, divergent one. This is
    /// not a verbatim passthrough of every push, though: `status --watch`
    /// normalizes each `state_changed` push into `Outcome::State` before it
    /// ever reaches here, so the `state_changed` tag itself never appears in
    /// `--json` output — every state, initial or pushed, is emitted as
    /// `{"event":"state",...}`.
    fn to_response(&self) -> Response {
        match self {
            Outcome::State(state) => Response::State { state: *state },
            Outcome::Config(cfg) => Response::Config(cfg.clone()),
            Outcome::Devices(devices) => Response::Devices {
                devices: devices.clone(),
            },
            Outcome::Ack => Response::Ok,
        }
    }
}

/// Render an outcome for the terminal, as JSON or in human form.
pub fn render(outcome: &Outcome, json: bool) -> Result<String> {
    if json {
        return serde_json::to_string(&outcome.to_response()).context("serializing JSON output");
    }
    Ok(match outcome {
        Outcome::State(state) => state.to_string(),
        Outcome::Ack => "ok".to_string(),
        Outcome::Config(cfg) => toml::to_string_pretty(&**cfg)
            .context("serializing config as TOML")?
            .trim_end()
            .to_string(),
        Outcome::Devices(devices) => render_devices(devices),
    })
}

fn render_devices(devices: &[InputDeviceInfo]) -> String {
    if devices.is_empty() {
        return "no input devices".to_string();
    }
    devices
        .iter()
        .map(|d| {
            let mut tags = Vec::new();
            if d.is_keyboard {
                tags.push("keyboard");
            }
            if d.is_pointer {
                tags.push("pointer");
            }
            let tags = if tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", tags.join(","))
            };
            format!("{}  {}{}", d.path.display(), d.name, tags)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn device(path: &str, name: &str, keyboard: bool, pointer: bool) -> InputDeviceInfo {
        InputDeviceInfo {
            path: PathBuf::from(path),
            name: name.to_string(),
            is_keyboard: keyboard,
            is_pointer: pointer,
        }
    }

    #[test]
    fn human_state_is_the_bare_label() {
        assert_eq!(
            render(&Outcome::State(GuardState::Armed), false).unwrap(),
            "Armed"
        );
    }

    #[test]
    fn human_ack_is_ok() {
        assert_eq!(render(&Outcome::Ack, false).unwrap(), "ok");
    }

    #[test]
    fn human_devices_list_path_name_and_tags() {
        let out = render(
            &Outcome::Devices(vec![
                device("/dev/input/event3", "Fake Keyboard", true, false),
                device("/dev/input/event4", "Fake Mouse", false, true),
            ]),
            false,
        )
        .unwrap();
        assert_eq!(
            out,
            "/dev/input/event3  Fake Keyboard [keyboard]\n/dev/input/event4  Fake Mouse [pointer]"
        );
    }

    #[test]
    fn human_devices_says_so_when_empty() {
        assert_eq!(
            render(&Outcome::Devices(vec![]), false).unwrap(),
            "no input devices"
        );
    }

    #[test]
    fn human_config_is_toml_that_round_trips() {
        let out = render(&Outcome::Config(Box::default()), false).unwrap();
        let back: Config = toml::from_str(&out).unwrap();
        assert_eq!(back, Config::default());
    }

    #[test]
    fn json_state_matches_the_wire_format() {
        let out = render(&Outcome::State(GuardState::Idle), true).unwrap();
        assert_eq!(out, r#"{"event":"state","state":"idle"}"#);
    }

    #[test]
    fn json_ack_matches_the_wire_format() {
        assert_eq!(render(&Outcome::Ack, true).unwrap(), r#"{"event":"ok"}"#);
    }
}
