//! Guided pairing: press a button, get a working remote.

use super::with_client;
use crate::error::CliError;
use crate::render::Outcome;
use std::path::Path;

/// Learn a remote by pressing one of its buttons, then save it.
///
/// The daemon only *reports* the press; writing it is an ordinary `SetConfig`
/// from here, exactly as the tray does. That keeps one write path for the
/// configuration no matter which front end paired the remote.
///
/// Blocks until a key arrives or the window closes, so it prints the prompt to
/// stderr first — stdout stays reserved for the result, and `--json` output stays
/// machine-readable.
pub fn run(socket: &Path, json: bool, timeout_secs: u64) -> Result<(), CliError> {
    with_client(socket, json, |client| {
        if !json {
            eprintln!("Press a button on your remote… ({timeout_secs}s)");
        }
        let Some(learned) = client.learn_remote(timeout_secs)? else {
            return Ok(Outcome::Paired(None));
        };
        let mut cfg = client.get_config()?;
        cfg.apply_learned_remote(&learned.path, &learned.name, &learned.key);
        client.set_config(cfg)?;
        Ok(Outcome::Paired(Some(learned)))
    })
}
