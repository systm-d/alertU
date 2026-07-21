//! AlertU per-session GUI: a StatusNotifierItem tray reflecting the daemon's
//! state, with device selection and settings driven through its menu.
//!
//! One Unix-socket connection carries everything: outgoing [`Request`]s from
//! menu callbacks (written by a small writer task) and incoming [`Response`]s
//! (state pushes, config, device lists) that update the tray via `Handle`.

mod tray;

use alertu_common::protocol::{Request, Response, DEFAULT_SOCKET_PATH};
use anyhow::{Context, Result};
use ksni::TrayMethods;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let socket = socket_path();
    let stream = UnixStream::connect(&socket).await.with_context(|| {
        format!(
            "connecting to daemon socket {} (is alertu-daemon running?)",
            socket.display()
        )
    })?;
    info!(socket = %socket.display(), "connected to daemon");

    let (read_half, mut write_half) = stream.into_split();
    let (req_tx, mut req_rx) = mpsc::unbounded_channel::<Request>();

    // Bring up the tray.
    let handle = AlertuTray::new(req_tx.clone(), socket.clone())
        .spawn()
        .await
        .context("spawning system tray (need a StatusNotifierItem host / D-Bus session)")?;
    info!("tray registered");

    // Writer task: serialize outgoing requests to the socket.
    tokio::spawn(async move {
        while let Some(req) = req_rx.recv().await {
            match serde_json::to_vec(&req) {
                Ok(mut buf) => {
                    buf.push(b'\n');
                    if let Err(e) = write_half.write_all(&buf).await {
                        warn!(error = %e, "failed to send request; closing writer");
                        break;
                    }
                    let _ = write_half.flush().await;
                }
                Err(e) => warn!(error = %e, "failed to serialize request"),
            }
        }
    });

    // Prime the tray with current state, config, and device list.
    let _ = req_tx.send(Request::Subscribe);
    let _ = req_tx.send(Request::GetConfig);
    let _ = req_tx.send(Request::ListDevices);

    // Reader loop: apply daemon responses to the tray model.
    let mut lines = BufReader::new(read_half).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Response>(&line) {
                    Ok(resp) => apply_response(&handle, resp).await,
                    Err(e) => warn!(error = %e, line = %line, "unparseable response"),
                }
            }
            Ok(None) => {
                error!("daemon closed the connection");
                break;
            }
            Err(e) => {
                error!(error = %e, "read error from daemon");
                break;
            }
        }
    }

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
