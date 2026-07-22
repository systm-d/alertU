//! Reading and changing the guard state: `arm`, `disarm`, `toggle`, `status`.

use super::{print_outcome, with_client};
use crate::error::CliError;
use crate::render::{self, Outcome};
use alertu_common::ipc_client::Client;
use alertu_common::protocol::Response;
use anyhow::{Context, Result};
use std::path::Path;

/// Force-arm: lock the session and start watching.
pub fn arm(socket: &Path, json: bool) -> Result<(), CliError> {
    with_client(socket, json, |client| {
        client.arm()?;
        Ok(Outcome::Ack)
    })
}

/// Force-disarm: unlock the session and stop watching.
pub fn disarm(socket: &Path, json: bool) -> Result<(), CliError> {
    with_client(socket, json, |client| {
        client.disarm()?;
        Ok(Outcome::Ack)
    })
}

/// Toggle arm/disarm, exactly as a remote button press would.
pub fn toggle(socket: &Path, json: bool) -> Result<(), CliError> {
    with_client(socket, json, |client| {
        client.toggle()?;
        Ok(Outcome::Ack)
    })
}

/// Print the current guard state, once.
pub fn status(socket: &Path, json: bool) -> Result<(), CliError> {
    with_client(socket, json, |client| {
        Ok(Outcome::State(client.get_state()?))
    })
}

/// `status --watch`: print the current state, then one line per push until the
/// connection errors out or the daemon reports an error.
///
/// Streams rather than producing a single outcome, which is why it does not go
/// through `with_client`.
pub fn watch(socket: &Path, json: bool) -> Result<(), CliError> {
    let mut client = Client::connect(socket)?;
    watch_states(&mut client, json)?;
    Ok(())
}

fn watch_states(client: &mut Client, json: bool) -> Result<()> {
    let state = client.subscribe()?;
    print_outcome(&Outcome::State(state), json)?;
    loop {
        let push = client.next_push()?;
        match push {
            // `--json` is contractually the raw `Response`, so a push keeps its
            // own `state_changed` tag: a consumer must be able to tell the
            // initial snapshot from a later transition. The human form is just
            // the bare state label either way.
            Response::State { state } | Response::StateChanged { state } => {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string(&push).context("serializing JSON output")?
                    );
                } else {
                    println!("{}", render::render(&Outcome::State(state), false)?);
                }
            }
            // Device-list pushes also arrive on a subscribed connection;
            // they are not state changes, so `status` ignores them.
            Response::Devices { .. } | Response::Config(_) | Response::Ok => {}
            Response::Error { message } => anyhow::bail!(message),
        }
    }
}
