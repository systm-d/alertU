//! AlertU daemon entrypoint.
//!
//! Wires together config, the evdev readers, the session monitor, the state
//! machine, and the IPC socket, then runs until `SIGINT`/`SIGTERM`.

mod devices;
mod input;
mod ipc;
mod machine;
mod session;
mod snapshot;
mod sound;
mod transitions;
mod webhook;

use alertu_common::config::Config;
use alertu_common::protocol::DEFAULT_SOCKET_PATH;
use alertu_common::state::GuardState;
use anyhow::{Context, Result};
use machine::{Channels, Control, Machine};
use session::SessionCtl;
use sound::SoundPlayer;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

/// Parsed command-line options.
struct Args {
    config: PathBuf,
    socket: PathBuf,
}

fn parse_args() -> Args {
    let mut config = Config::default_path();
    let mut socket = PathBuf::from(DEFAULT_SOCKET_PATH);
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--config" | "-c" => {
                if let Some(v) = it.next() {
                    config = PathBuf::from(v);
                }
            }
            "--socket" | "-s" => {
                if let Some(v) = it.next() {
                    socket = PathBuf::from(v);
                }
            }
            "--help" | "-h" => {
                println!(
                    "alertu-daemon [--config <path>] [--socket <path>]\n\n\
                     Defaults:\n  config: {}\n  socket: {}",
                    Config::default_path().display(),
                    DEFAULT_SOCKET_PATH
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("warning: ignoring unknown argument {other}");
            }
        }
    }
    Args { config, socket }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = parse_args();

    let cfg = Config::load(&args.config)
        .with_context(|| format!("loading config {}", args.config.display()))?;
    cfg.validate().context("validating config")?;
    info!(config = %args.config.display(), "loaded config");

    // Channels: input signals, lock-state changes, control commands, state.
    let (sig_tx, sig_rx) = mpsc::channel(64);
    let (lock_tx, lock_rx) = mpsc::channel(16);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<Control>(32);
    let (state_tx, state_rx) = watch::channel(GuardState::Idle);

    let session = SessionCtl::new(&cfg).await;
    info!(session = %session.id(), "controlling logind session");
    let sound = SoundPlayer::new();

    // Session lock-state monitor (detects password unlock while armed).
    tokio::spawn(session::monitor(
        session.clone(),
        lock_tx,
        Duration::from_millis(500),
    ));

    // State machine.
    let machine = Machine::new(
        cfg,
        args.config.clone(),
        session,
        sound,
        Channels {
            state_tx,
            sig_tx,
            sig_rx,
            lock_rx,
            ctrl_rx,
        },
    );
    let machine_handle = tokio::spawn(machine.run());

    // IPC server.
    let socket_path = args.socket.clone();
    let ipc_handle = tokio::spawn(ipc::serve(socket_path.clone(), state_rx, ctrl_tx));

    // Wait for a shutdown signal.
    shutdown_signal().await;
    info!("shutting down");
    ipc::cleanup(&socket_path);
    ipc_handle.abort();
    machine_handle.abort();
    Ok(())
}

/// Resolve when the process receives SIGINT or SIGTERM.
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "cannot install SIGTERM handler");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = term.recv() => {},
    }
}
