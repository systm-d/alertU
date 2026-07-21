//! Library face of the AlertU daemon.
//!
//! The binary in `main.rs` is a thin wiring layer over these modules. They are
//! public so integration tests can drive the state machine in-process, without
//! evdev devices or any privilege.

pub mod devices;
pub mod hotplug;
pub mod input;
pub mod ipc;
pub mod machine;
pub mod session;
pub mod snapshot;
pub mod sound;
pub mod transitions;
pub mod webhook;
