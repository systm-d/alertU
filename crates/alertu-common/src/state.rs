//! The guard state machine states, shared between daemon and GUI.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The high-level state of the alarm.
///
/// Lifecycle: `Idle → Armed → Triggered → Alarm → Idle`.
///
/// * `Idle`      — disarmed, nothing watched.
/// * `Armed`     — session locked, watched input devices are monitored for intrusion.
/// * `Triggered` — intrusion detected while armed; a countdown to the siren is running.
/// * `Alarm`     — countdown elapsed without disarm; siren looping and snapshot taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardState {
    Idle,
    Armed,
    Triggered,
    Alarm,
}

impl GuardState {
    /// Whether the alarm is "engaged" in any way (anything other than `Idle`).
    pub fn is_active(self) -> bool {
        !matches!(self, GuardState::Idle)
    }

    /// A short human label, handy for the tray tooltip / logs.
    pub fn label(self) -> &'static str {
        match self {
            GuardState::Idle => "Idle",
            GuardState::Armed => "Armed",
            GuardState::Triggered => "Triggered",
            GuardState::Alarm => "Alarm",
        }
    }
}

impl fmt::Display for GuardState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_idle_is_inactive() {
        assert!(!GuardState::Idle.is_active());
        assert!(GuardState::Armed.is_active());
        assert!(GuardState::Triggered.is_active());
        assert!(GuardState::Alarm.is_active());
    }

    #[test]
    fn serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&GuardState::Alarm).unwrap(),
            "\"alarm\""
        );
        let back: GuardState = serde_json::from_str("\"armed\"").unwrap();
        assert_eq!(back, GuardState::Armed);
    }
}
