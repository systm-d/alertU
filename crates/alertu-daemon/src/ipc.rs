//! Unix-socket IPC server: newline-delimited JSON, one connection per GUI.

use crate::devices;
use crate::machine::Control;
use alertu_common::protocol::{Request, Response};
use alertu_common::state::GuardState;
use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{debug, info, warn};

/// Bind the socket and accept connections until cancelled.
///
/// The socket is created with `0o666` so a per-session GUI running as the
/// desktop user can connect to the root-owned daemon. This is a personal
/// gadget, not a hardened multi-user service (see the README's threat-model
/// note); tighten via directory permissions if needed.
pub async fn serve(
    socket_path: PathBuf,
    state_rx: watch::Receiver<GuardState>,
    ctrl_tx: mpsc::Sender<Control>,
) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating socket dir {}", parent.display()))?;
    }
    // Remove a stale socket from a previous run.
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("binding socket {}", socket_path.display()))?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o666))
        .with_context(|| format!("setting perms on {}", socket_path.display()))?;
    info!(socket = %socket_path.display(), "IPC listening");

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let state_rx = state_rx.clone();
                let ctrl_tx = ctrl_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(stream, state_rx, ctrl_tx).await {
                        debug!(error = %e, "client connection ended");
                    }
                });
            }
            Err(e) => warn!(error = %e, "accept failed"),
        }
    }
}

async fn handle_conn(
    stream: UnixStream,
    mut state_rx: watch::Receiver<GuardState>,
    ctrl_tx: mpsc::Sender<Control>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();
    let mut subscribed = false;

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let response = match serde_json::from_str::<Request>(&line) {
                    Ok(req) => dispatch(req, &state_rx, &ctrl_tx, &mut subscribed).await,
                    Err(e) => Response::Error { message: format!("bad request: {e}") },
                };
                write_response(&mut write_half, &response).await?;
            }
            changed = state_rx.changed(), if subscribed => {
                if changed.is_err() {
                    break; // sender dropped
                }
                let state = *state_rx.borrow();
                write_response(&mut write_half, &Response::StateChanged { state }).await?;
            }
        }
    }
    Ok(())
}

async fn dispatch(
    req: Request,
    state_rx: &watch::Receiver<GuardState>,
    ctrl_tx: &mpsc::Sender<Control>,
    subscribed: &mut bool,
) -> Response {
    match req {
        Request::GetState => Response::State {
            state: *state_rx.borrow(),
        },
        Request::Subscribe => {
            *subscribed = true;
            Response::State {
                state: *state_rx.borrow(),
            }
        }
        Request::Arm => send_ctrl(ctrl_tx, Control::Arm).await,
        Request::Disarm => send_ctrl(ctrl_tx, Control::Disarm).await,
        Request::Toggle => send_ctrl(ctrl_tx, Control::Toggle).await,
        Request::GetConfig => {
            let (tx, rx) = oneshot::channel();
            if ctrl_tx.send(Control::GetConfig(tx)).await.is_err() {
                return daemon_gone();
            }
            match rx.await {
                Ok(cfg) => Response::Config(Box::new(cfg)),
                Err(_) => daemon_gone(),
            }
        }
        Request::SetConfig(cfg) => {
            let (tx, rx) = oneshot::channel();
            if ctrl_tx.send(Control::SetConfig(cfg, tx)).await.is_err() {
                return daemon_gone();
            }
            match rx.await {
                Ok(Ok(())) => Response::Ok,
                Ok(Err(msg)) => Response::Error { message: msg },
                Err(_) => daemon_gone(),
            }
        }
        Request::ListDevices => {
            let devices = tokio::task::spawn_blocking(|| {
                devices::list().iter().map(|e| e.to_info()).collect()
            })
            .await
            .unwrap_or_default();
            Response::Devices { devices }
        }
    }
}

/// Fire-and-forget control command that just needs an ack.
async fn send_ctrl(ctrl_tx: &mpsc::Sender<Control>, ctrl: Control) -> Response {
    if ctrl_tx.send(ctrl).await.is_err() {
        daemon_gone()
    } else {
        Response::Ok
    }
}

fn daemon_gone() -> Response {
    Response::Error {
        message: "daemon state machine unavailable".to_string(),
    }
}

async fn write_response<W>(writer: &mut W, response: &Response) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut buf = serde_json::to_vec(response).context("serializing response")?;
    buf.push(b'\n');
    writer.write_all(&buf).await.context("writing response")?;
    writer.flush().await.context("flushing response")?;
    Ok(())
}

/// Best-effort socket removal on shutdown.
pub fn cleanup(socket_path: &Path) {
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
}
