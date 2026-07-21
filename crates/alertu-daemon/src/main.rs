//! AlertU daemon entrypoint.
//!
//! Wires together config, the evdev readers, the session monitor, the state
//! machine, and the IPC socket, then runs until `SIGINT`/`SIGTERM`.

#![deny(unsafe_code)]

use alertu_common::config::Config;
use alertu_common::protocol::DEFAULT_SOCKET_PATH;
use alertu_common::state::GuardState;
use alertu_daemon::machine::{Channels, Control, Machine};
use alertu_daemon::perms::{self, Privileges};
use alertu_daemon::session::{self, SessionCtl};
use alertu_daemon::sound::SoundPlayer;
use alertu_daemon::{hotplug, ipc};
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

/// Parsed command-line options.
struct Args {
    config: PathBuf,
    socket: PathBuf,
    /// Not a `Config` field: it is a command-line flag precisely so it cannot
    /// be changed over the very socket it protects.
    socket_group: Option<String>,
}

/// The value that must follow `flag`.
///
/// A flag whose value is missing is a hard error, never a silent default: every
/// one of these three changes what the daemon exposes and to whom, so quietly
/// ignoring `--socket-group` (leaving the socket on the daemon's own group) or
/// `--config` (running with settings nobody asked for) is exactly the failure
/// mode worth refusing to start over. A following word that itself looks like a
/// flag is the same mistake spelled differently — `--socket-group --socket /p`
/// would otherwise report "no such group: --socket" and drop the real path.
fn value_for(flag: &str, it: &mut impl Iterator<Item = String>) -> Result<String> {
    match it.next() {
        Some(v) if v.starts_with('-') && v.len() > 1 => Err(anyhow!(
            "{flag} expects a value, but the next argument is another flag: {v}"
        )),
        Some(v) => Ok(v),
        None => Err(anyhow!("{flag} expects a value")),
    }
}

fn parse_args() -> Result<Args> {
    let mut config = Config::default_path();
    let mut socket = PathBuf::from(DEFAULT_SOCKET_PATH);
    let mut socket_group = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--config" | "-c" => config = PathBuf::from(value_for(&arg, &mut it)?),
            "--socket" | "-s" => socket = PathBuf::from(value_for(&arg, &mut it)?),
            "--socket-group" => socket_group = Some(value_for(&arg, &mut it)?),
            "--help" | "-h" => {
                println!(
                    "alertu-daemon [--config <path>] [--socket <path>] [--socket-group <name>]\n\n\
                     Defaults:\n  config: {}\n  socket: {}\n  socket group: the daemon's own group\n\n\
                     The control socket is created 0660. Members of its group get full\n\
                     control of the alarm; treat membership as a privilege grant.",
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
    Ok(Args {
        config,
        socket,
        socket_group,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = parse_args().context("parsing command-line arguments")?;

    // Resolved up front: an unknown group must abort startup, never silently
    // leave a socket more permissive than the operator asked for.
    let privileges = Privileges {
        group_gid: match &args.socket_group {
            Some(name) => Some(
                perms::resolve_gid(name)
                    .with_context(|| format!("resolving --socket-group {name}"))?,
            ),
            None => None,
        },
    };

    let cfg = Config::load(&args.config)
        .with_context(|| format!("loading config {}", args.config.display()))?;
    cfg.validate().context("validating config")?;
    info!(config = %args.config.display(), "loaded config");

    // Channels: input signals, lock-state changes, control commands, state,
    // and the current device list (pushed to subscribers on hotplug).
    let (sig_tx, sig_rx) = mpsc::channel(64);
    let (lock_tx, lock_rx) = mpsc::channel(16);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<Control>(32);
    let (state_tx, state_rx) = watch::channel(GuardState::Idle);
    let (devices_tx, devices_rx) = watch::channel(Vec::new());

    let session = SessionCtl::new(&cfg).await;
    info!(session = %session.id(), "controlling logind session");
    let sound = SoundPlayer::new();

    // Session lock-state monitor (detects password unlock while armed).
    tokio::spawn(session::monitor(
        session.clone(),
        lock_tx,
        Duration::from_millis(500),
    ));

    // Device hotplug watcher (re-scans on /dev/input changes).
    tokio::spawn(hotplug::watch(hotplug::INPUT_DIR, ctrl_tx.clone()));

    // State machine.
    let machine = Machine::new(
        cfg,
        args.config.clone(),
        session,
        sound,
        privileges,
        Channels {
            state_tx,
            devices_tx,
            sig_tx,
            sig_rx,
            lock_rx,
            ctrl_rx,
        },
    );
    let machine_handle = tokio::spawn(machine.run());

    // IPC server. Binding happens here, not inside the spawned task, so a
    // failure aborts startup loudly instead of leaving an uncontrollable daemon
    // running with nothing in the log.
    let socket_path = args.socket.clone();
    let listener = ipc::bind(&socket_path, privileges)
        .with_context(|| format!("binding the control socket {}", socket_path.display()))?;
    let ipc_handle = tokio::spawn(ipc::serve(listener, state_rx, devices_rx, ctrl_tx));

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
