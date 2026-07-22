//! Listing the input devices the daemon can see.

use super::with_client;
use crate::error::CliError;
use crate::render::Outcome;
use std::path::Path;

/// List the input devices the daemon can see.
///
/// The daemon is the component with `input`-group access, so it owns device
/// discovery and reports the list over IPC — this command never touches
/// `/dev/input` itself.
pub fn list(socket: &Path, json: bool) -> Result<(), CliError> {
    with_client(socket, json, |client| {
        Ok(Outcome::Devices(client.list_devices()?))
    })
}
