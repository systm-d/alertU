//! The pure core of the guard state machine.
//!
//! [`decide`] is a side-effect-free function mapping `(state, event)` to the
//! next state plus the list of [`Effect`]s the interpreter must carry out. All
//! I/O (session locking, audio, snapshots, timers) lives in the interpreter
//! ([`crate::machine`]); keeping the decision logic pure makes the whole
//! transition table exhaustively unit-testable.

use alertu_common::state::GuardState;

/// A stimulus the machine reacts to. Timing details (grace elapsed, which timer
/// fired) are resolved by the interpreter before it emits one of these, so the
/// table below stays purely about *what* happens, not *when*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Remote button, or an IPC `Toggle`: arm when idle, disarm otherwise.
    Toggle,
    /// IPC forced arm (no-op unless idle).
    ForceArm,
    /// IPC forced disarm (no-op when already idle).
    ForceDisarm,
    /// Activity on a watched device, already confirmed past the grace period.
    Intrusion,
    /// The session was unlocked out-of-band (e.g. password) — disarms without
    /// us issuing an unlock, since it is already unlocked.
    SessionUnlocked,
    /// The `Triggered` countdown reached zero.
    CountdownElapsed,
}

/// A side effect the interpreter performs as part of a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// `loginctl lock-session`.
    LockSession,
    /// `loginctl unlock-session`.
    UnlockSession,
    /// Play the arm/disarm chirp once.
    PlayBeep,
    /// Start the looping siren.
    StartSiren,
    /// Stop the looping siren.
    StopSiren,
    /// Capture a webcam snapshot.
    Snapshot,
    /// Fire the optional alarm webhook.
    Webhook,
    /// Begin the post-arm grace window before intrusion detection goes live.
    StartGrace,
    /// Begin the `Triggered` countdown (and its warning ticks).
    StartCountdown,
    /// Clear all pending timers (grace, countdown, warning).
    ClearTimers,
}

/// The result of a transition: the next state and the effects to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub next: GuardState,
    pub effects: Vec<Effect>,
}

impl Transition {
    fn new(next: GuardState, effects: Vec<Effect>) -> Self {
        Transition { next, effects }
    }
}

/// Decide the transition for `(state, event)`, or `None` if the event should be
/// ignored in the current state.
///
/// Disarm has two flavours, which is the whole reason unlock detection is
/// correct: a `Toggle`/`ForceDisarm` actively unlocks the session, while a
/// `SessionUnlocked` event (the user already unlocked) must *not* re-issue an
/// unlock — it only tears down alarm state.
pub fn decide(state: GuardState, event: Event) -> Option<Transition> {
    use Effect::*;
    use Event::*;
    use GuardState::*;

    match (state, event) {
        // --- arming -------------------------------------------------------
        (Idle, Toggle) | (Idle, ForceArm) => Some(Transition::new(
            Armed,
            vec![LockSession, PlayBeep, StartGrace],
        )),

        // --- disarming by remote / force (we unlock the session) ----------
        (s, Toggle) | (s, ForceDisarm) if s.is_active() => Some(Transition::new(
            Idle,
            vec![UnlockSession, StopSiren, ClearTimers, PlayBeep],
        )),

        // --- disarming by external unlock (session already unlocked) ------
        (s, SessionUnlocked) if s.is_active() => Some(Transition::new(
            Idle,
            vec![StopSiren, ClearTimers, PlayBeep],
        )),

        // --- intrusion ----------------------------------------------------
        (Armed, Intrusion) => Some(Transition::new(Triggered, vec![StartCountdown])),

        // --- countdown expiry ---------------------------------------------
        (Triggered, CountdownElapsed) => Some(Transition::new(
            Alarm,
            vec![ClearTimers, StartSiren, Snapshot, Webhook],
        )),

        // Everything else is a no-op in the current state.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::Effect::*;
    use super::*;
    use GuardState::*;

    fn t(state: GuardState, event: Event) -> Option<Transition> {
        decide(state, event)
    }

    #[test]
    fn toggle_from_idle_arms_and_locks() {
        let tr = t(Idle, Event::Toggle).unwrap();
        assert_eq!(tr.next, Armed);
        assert_eq!(tr.effects, vec![LockSession, PlayBeep, StartGrace]);
    }

    #[test]
    fn force_arm_only_from_idle() {
        assert_eq!(t(Idle, Event::ForceArm).unwrap().next, Armed);
        assert!(t(Armed, Event::ForceArm).is_none());
        assert!(t(Triggered, Event::ForceArm).is_none());
        assert!(t(Alarm, Event::ForceArm).is_none());
    }

    #[test]
    fn toggle_disarms_from_every_active_state_and_unlocks() {
        for s in [Armed, Triggered, Alarm] {
            let tr = t(s, Event::Toggle).unwrap();
            assert_eq!(tr.next, Idle, "from {s}");
            assert!(tr.effects.contains(&UnlockSession), "from {s} must unlock");
            assert!(tr.effects.contains(&StopSiren));
            assert!(tr.effects.contains(&ClearTimers));
        }
    }

    #[test]
    fn external_unlock_disarms_without_reissuing_unlock() {
        for s in [Armed, Triggered, Alarm] {
            let tr = t(s, Event::SessionUnlocked).unwrap();
            assert_eq!(tr.next, Idle, "from {s}");
            assert!(
                !tr.effects.contains(&UnlockSession),
                "external unlock must NOT call loginctl unlock (from {s})"
            );
            assert!(tr.effects.contains(&StopSiren));
        }
    }

    #[test]
    fn external_unlock_ignored_when_idle() {
        assert!(t(Idle, Event::SessionUnlocked).is_none());
    }

    #[test]
    fn intrusion_only_triggers_from_armed() {
        let tr = t(Armed, Event::Intrusion).unwrap();
        assert_eq!(tr.next, Triggered);
        assert_eq!(tr.effects, vec![StartCountdown]);

        assert!(t(Idle, Event::Intrusion).is_none());
        // Already past Armed: intrusion is redundant, not a fresh transition.
        assert!(t(Triggered, Event::Intrusion).is_none());
        assert!(t(Alarm, Event::Intrusion).is_none());
    }

    #[test]
    fn countdown_elapsed_fires_alarm_only_from_triggered() {
        let tr = t(Triggered, Event::CountdownElapsed).unwrap();
        assert_eq!(tr.next, Alarm);
        assert!(tr.effects.contains(&StartSiren));
        assert!(tr.effects.contains(&Snapshot));
        assert!(tr.effects.contains(&Webhook));

        assert!(t(Idle, Event::CountdownElapsed).is_none());
        assert!(t(Armed, Event::CountdownElapsed).is_none());
        assert!(t(Alarm, Event::CountdownElapsed).is_none());
    }

    #[test]
    fn force_disarm_is_noop_when_idle() {
        assert!(t(Idle, Event::ForceDisarm).is_none());
    }

    #[test]
    fn full_happy_path_sequence() {
        // Idle -arm-> Armed -intrusion-> Triggered -countdown-> Alarm -toggle-> Idle
        let mut state = Idle;
        for (event, expected) in [
            (Event::Toggle, Armed),
            (Event::Intrusion, Triggered),
            (Event::CountdownElapsed, Alarm),
            (Event::Toggle, Idle),
        ] {
            state = decide(state, event).unwrap().next;
            assert_eq!(state, expected);
        }
    }
}
