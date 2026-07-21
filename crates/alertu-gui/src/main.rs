//! AlertU per-session GUI: a StatusNotifierItem tray reflecting the daemon's
//! state, with device selection and settings driven through its menu.
//!
//! One Unix-socket connection carries everything: outgoing [`Request`]s that
//! menu callbacks queue on a channel, and incoming [`Response`]s (state pushes,
//! config, device lists) that update the tray via `Handle`. Both directions are
//! driven by the same `select!` inside [`run_session`] — there is no separate
//! writer task.
//!
//! The tray itself is spawned once and outlives every connection: the
//! supervisor loop in `main` reconnects to the daemon with exponential backoff,
//! so restarting the daemon never makes the tray icon disappear. While there is
//! no connection, queued requests are dropped with a warning rather than
//! replayed later against a state the user can no longer see.

mod tray;

use alertu_common::protocol::{Request, Response, DEFAULT_SOCKET_PATH};
use anyhow::{Context, Result};
use ksni::TrayMethods;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tracing::{info, warn};
use tray::AlertuTray;

fn socket_path() -> PathBuf {
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        if matches!(arg.as_str(), "--socket" | "-s") {
            if let Some(v) = it.next() {
                return PathBuf::from(v);
            }
        }
    }
    PathBuf::from(DEFAULT_SOCKET_PATH)
}

/// First retry delay after losing the daemon.
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
/// Ceiling for the exponential backoff.
const MAX_BACKOFF: Duration = Duration::from_secs(10);

/// The next retry delay: double, capped.
fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(MAX_BACKOFF)
}

/// What one connection attempt achieved, so the supervisor knows whether to
/// reset its backoff.
struct SessionOutcome {
    /// True once any response arrived. Connecting alone is not enough: a socket
    /// that accepts and immediately drops would otherwise never back off.
    got_response: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let socket = socket_path();
    let (req_tx, mut req_rx) = mpsc::unbounded_channel::<Request>();

    // The tray is spawned once and outlives every connection, so the icon never
    // disappears while the daemon restarts.
    let handle = AlertuTray::new(req_tx.clone(), socket.clone())
        .spawn()
        .await
        .context("spawning system tray (need a StatusNotifierItem host / D-Bus session)")?;
    info!("tray registered");

    let mut backoff = INITIAL_BACKOFF;
    loop {
        match run_session(&socket, &mut req_rx, &handle).await {
            Ok(outcome) => {
                if outcome.got_response {
                    backoff = INITIAL_BACKOFF;
                }
            }
            Err(e) => warn!(error = %e, "could not reach the daemon"),
        }
        handle.update(|t| t.connected = false).await;
        warn!(retry_in = ?backoff, "disconnected from daemon");
        tokio::time::sleep(backoff).await;
        backoff = next_backoff(backoff);
    }
}

/// Hold one connection to the daemon until it drops.
async fn run_session(
    socket: &Path,
    req_rx: &mut mpsc::UnboundedReceiver<Request>,
    handle: &ksni::Handle<AlertuTray>,
) -> Result<SessionOutcome> {
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting to daemon socket {}", socket.display()))?;
    info!(socket = %socket.display(), "connected to daemon");

    // Discard anything the menu queued while we were offline. Replaying a
    // ten-second-old `Arm` against a live alarm would be dangerous, and the
    // unbounded channel would otherwise deliver it now.
    let mut discarded = 0usize;
    while req_rx.try_recv().is_ok() {
        discarded += 1;
    }
    if discarded > 0 {
        warn!(discarded, "dropped requests queued while disconnected");
    }

    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();
    let mut outcome = SessionOutcome {
        got_response: false,
    };

    handle.update(|t| t.connected = true).await;

    // Resynchronise: state pushes, the config, and the device list.
    for req in [Request::Subscribe, Request::GetConfig, Request::ListDevices] {
        write_request(&mut write_half, &req).await?;
    }

    loop {
        tokio::select! {
            maybe = req_rx.recv() => {
                let Some(req) = maybe else {
                    // Every sender is gone. Unreachable today: `req_tx` is
                    // cloned into the tray, which lives as long as the process,
                    // and "Quit" calls `std::process::exit(0)` outright. Ending
                    // the session is the honest answer anyway — the supervisor
                    // will simply try to reconnect.
                    return Ok(outcome);
                };
                if let Err(e) = write_request(&mut write_half, &req).await {
                    warn!(error = %e, "dropping request; the connection is gone");
                    return Ok(outcome);
                }
            }
            line = lines.next_line() => match line {
                Ok(Some(l)) if l.trim().is_empty() => {}
                Ok(Some(l)) => {
                    outcome.got_response = true;
                    match serde_json::from_str::<Response>(&l) {
                        Ok(resp) => apply_response(handle, resp).await,
                        Err(e) => warn!(error = %e, line = %l, "unparseable response"),
                    }
                }
                Ok(None) => {
                    info!("daemon closed the connection");
                    return Ok(outcome);
                }
                Err(e) => {
                    warn!(error = %e, "read error from daemon");
                    return Ok(outcome);
                }
            },
        }
    }
}

/// Serialize one request onto the socket.
async fn write_request<W>(writer: &mut W, req: &Request) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut buf = serde_json::to_vec(req).context("serializing request")?;
    buf.push(b'\n');
    writer.write_all(&buf).await.context("writing request")?;
    writer.flush().await.context("flushing request")?;
    Ok(())
}

async fn apply_response(handle: &ksni::Handle<AlertuTray>, resp: Response) {
    match resp {
        Response::State { state } | Response::StateChanged { state } => {
            handle.update(move |t| t.state = state).await;
        }
        Response::Config(cfg) => {
            handle.update(move |t| t.cfg = Some(*cfg)).await;
        }
        Response::Devices { devices } => {
            handle.update(move |t| t.devices = devices).await;
        }
        Response::Ok => {}
        Response::Error { message } => warn!(%message, "daemon reported an error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Doubling from the initial delay up to the ceiling, which must then be
    /// sticky: a cap that is only applied once would let the delay keep growing
    /// past ten seconds and leave the tray offline for minutes.
    #[test]
    fn the_backoff_doubles_then_sticks_at_the_ceiling() {
        let mut d = INITIAL_BACKOFF;
        assert_eq!(d, Duration::from_millis(250));
        for expected in [
            Duration::from_millis(500),
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            MAX_BACKOFF,
            MAX_BACKOFF,
        ] {
            d = next_backoff(d);
            assert_eq!(d, expected);
        }
    }
}
