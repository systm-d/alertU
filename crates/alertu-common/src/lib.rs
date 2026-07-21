//! Types shared by the AlertU daemon and the per-session GUI client.

#![forbid(unsafe_code)]

pub mod config;
pub mod protocol;
pub mod state;

#[cfg(feature = "ipc-client")]
pub mod ipc_client;

pub use config::Config;
pub use protocol::{InputDeviceInfo, Request, Response, DEFAULT_SOCKET_PATH};
pub use state::GuardState;
