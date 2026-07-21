//! Unix-socket IPC server: newline-delimited JSON, one connection per GUI.

use crate::machine::Control;
use crate::perms::{self, Privileges};
use alertu_common::protocol::{InputDeviceInfo, Request, Response};
use alertu_common::state::GuardState;
use anyhow::{Context, Result};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{debug, info, warn};

/// Bind the control socket, replacing a stale one from a previous run.
///
/// This is deliberately separate from [`serve`] and synchronous: binding is the
/// one part that can fail, and every front end (tray, settings window,
/// `alertu-ctl`) depends on the socket. Doing it before the daemon claims to be
/// up means a failure — an over-long path, a bad directory, missing permissions
/// — aborts startup with a diagnostic instead of leaving a daemon running with
/// no way to control it.
///
/// The socket is created `0o660`: reachable by the daemon's own user and group,
/// never by everyone else. Connecting grants full command of the alarm —
/// disarm, read the config including the webhook URL, `SetConfig` to redirect
/// the paths handed to the helper programs — so group membership is a
/// privilege grant, not a convenience. Any failure here aborts startup with a
/// diagnostic.
pub fn bind(socket_path: &Path, privileges: Privileges) -> Result<UnixListener> {
    let parent = socket_path.parent();

    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating socket dir {}", parent.display()))?;
        // Tighten the parent directory *before* binding, not only when an
        // explicit group is given. `bind` and the `chmod` that follows it below
        // are two separate syscalls, and between them the socket briefly sits at
        // whatever mode the process umask produced — under a permissive umask
        // that is a real window in which a non-group member can connect and
        // issue commands (e.g. `disarm`). A `0750` parent closes that window
        // regardless of umask: nothing can traverse into the directory to reach
        // the socket at all, whatever the socket's own mode happens to be at
        // that instant.
        perms::chmod(parent, 0o750)
            .with_context(|| format!("setting mode on socket dir {}", parent.display()))?;
    }

    // Remove a stale socket from a previous run.
    if socket_path.exists() {
        std::fs::remove_file(socket_path)
            .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
    }

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("binding socket {}", socket_path.display()))?;
    perms::chmod(socket_path, 0o660)?;

    // With an explicit group, the parent directory must carry it too, or the
    // group cannot traverse into the socket and the flag is silently
    // inoperative. systemd recreates this directory on every start, so the
    // change does not persist.
    if let Some(gid) = privileges.group_gid {
        perms::chgrp(socket_path, gid)?;
        if let Some(parent) = parent {
            perms::chgrp(parent, gid)?;
        }
    }

    info!(socket = %socket_path.display(), "IPC listening");
    Ok(listener)
}

/// Accept connections on an already-bound listener, until cancelled.
pub async fn serve(
    listener: UnixListener,
    state_rx: watch::Receiver<GuardState>,
    devices_rx: watch::Receiver<Vec<InputDeviceInfo>>,
    ctrl_tx: mpsc::Sender<Control>,
) -> Result<()> {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let state_rx = state_rx.clone();
                let devices_rx = devices_rx.clone();
                let ctrl_tx = ctrl_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(stream, state_rx, devices_rx, ctrl_tx).await {
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
    mut devices_rx: watch::Receiver<Vec<InputDeviceInfo>>,
    ctrl_tx: mpsc::Sender<Control>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();
    let mut subscribed = false;

    // These receivers are clones of the long-lived ones held by `serve`, which
    // never marks anything seen, so they start out flagged as "changed" as soon
    // as the machine has published anything at all. Left alone, the very first
    // `changed()` below would resolve immediately and push a duplicate of the
    // snapshot the `Subscribe` reply already carries — a phantom transition for
    // anything reading `status --watch`. Mark them current: subscribers only
    // want what happens *after* they subscribe (the tray asks for `ListDevices`
    // explicitly at startup).
    state_rx.mark_unchanged();
    devices_rx.mark_unchanged();

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let response = match serde_json::from_str::<Request>(&line) {
                    Ok(req) => dispatch(req, &state_rx, &devices_rx, &ctrl_tx, &mut subscribed).await,
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
            changed = devices_rx.changed(), if subscribed => {
                if changed.is_err() {
                    break; // sender dropped
                }
                let devices = devices_rx.borrow().clone();
                write_response(&mut write_half, &Response::Devices { devices }).await?;
            }
        }
    }
    Ok(())
}

async fn dispatch(
    req: Request,
    state_rx: &watch::Receiver<GuardState>,
    devices_rx: &watch::Receiver<Vec<InputDeviceInfo>>,
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
        Request::ListDevices => Response::Devices {
            devices: devices_rx.borrow().clone(),
        },
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
