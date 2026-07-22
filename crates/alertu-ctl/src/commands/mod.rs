//! One module per group of subcommands.
//!
//! Each exposes plain functions taking only what they need — the socket path,
//! whether to emit JSON — rather than the parsed `Cli`. That keeps the
//! dependency one-way: `cli` knows about `commands`, never the reverse.

pub mod config;
pub mod devices;
pub mod gen_sounds;
pub mod state;

use crate::error::CliError;
use crate::render::{self, Outcome};
use alertu_common::ipc_client::Client;
use anyhow::Result;
use std::path::Path;

/// Connect, run a single request/response exchange, and print the result.
/// Shared by every command that needs nothing more than one round trip.
pub(crate) fn with_client(
    socket: &Path,
    json: bool,
    f: impl FnOnce(&mut Client) -> Result<Outcome>,
) -> Result<(), CliError> {
    let mut client = Client::connect(socket)?;
    let outcome = f(&mut client)?;
    print_outcome(&outcome, json)?;
    Ok(())
}

pub(crate) fn print_outcome(outcome: &Outcome, json: bool) -> Result<()> {
    println!("{}", render::render(outcome, json)?);
    Ok(())
}
