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

#![forbid(unsafe_code)]

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
        handle.set_connected(false).await;
        warn!(retry_in = ?backoff, "disconnected from daemon");
        tokio::time::sleep(backoff).await;
        backoff = next_backoff(backoff);
    }
}

/// What a session does to the UI.
///
/// A seam, not a redesign. It exists so `run_session` can be driven in tests
/// against a recording fake: `ksni::TrayMethods::spawn` always registers with a
/// live `StatusNotifierWatcher`, so there is no way to exercise the reconnection
/// logic through the real tray.
trait TrayView {
    async fn set_connected(&self, connected: bool);
    async fn apply(&self, response: Response);
}

impl TrayView for ksni::Handle<AlertuTray> {
    async fn set_connected(&self, connected: bool) {
        self.update(move |t| t.connected = connected).await;
    }

    async fn apply(&self, response: Response) {
        apply_response(self, response).await;
    }
}

/// Hold one connection to the daemon until it drops.
async fn run_session(
    socket: &Path,
    req_rx: &mut mpsc::UnboundedReceiver<Request>,
    view: &impl TrayView,
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

    view.set_connected(true).await;

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
                        Ok(resp) => view.apply(resp).await,
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
    use alertu_common::state::GuardState;
    use std::sync::Mutex;
    use tokio::net::UnixListener;

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

    /// Records what a session did to the UI, standing in for the real tray.
    #[derive(Default)]
    struct RecordingView {
        connected: Mutex<Vec<bool>>,
        applied: Mutex<Vec<Response>>,
    }

    impl TrayView for RecordingView {
        async fn set_connected(&self, connected: bool) {
            self.connected.lock().unwrap().push(connected);
        }
        async fn apply(&self, response: Response) {
            self.applied.lock().unwrap().push(response);
        }
    }

    /// A fake daemon that reads `expect` request lines, optionally answers, then
    /// hangs up. Returns the requests it saw.
    ///
    /// After `expect` lines, it waits a brief grace period for one more before
    /// hanging up: the resync requests always reach the socket first, so a
    /// stray request that a regression replayed anyway would otherwise still
    /// be sitting unread when the daemon closes the connection, silently
    /// passing tests that exist specifically to catch that replay.
    async fn fake_daemon(
        listener: UnixListener,
        expect: usize,
        replies: Vec<&'static str>,
    ) -> Vec<Request> {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();

        let mut seen = Vec::new();
        for reply in replies {
            use tokio::io::AsyncWriteExt as _;
            write_half.write_all(reply.as_bytes()).await.unwrap();
            write_half.write_all(b"\n").await.unwrap();
        }
        while seen.len() < expect {
            match lines.next_line().await.unwrap() {
                Some(l) if l.trim().is_empty() => {}
                Some(l) => seen.push(serde_json::from_str(&l).unwrap()),
                None => break,
            }
        }
        if let Ok(Ok(Some(l))) =
            tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await
        {
            if let Ok(req) = serde_json::from_str(&l) {
                seen.push(req);
            }
        }
        seen
    }

    #[tokio::test]
    async fn a_session_resynchronises_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("s.sock");
        let listener = UnixListener::bind(&socket).unwrap();

        let (_tx, mut rx) = mpsc::unbounded_channel::<Request>();
        let view = RecordingView::default();

        let daemon = tokio::spawn(fake_daemon(listener, 3, vec![]));
        let session = run_session(&socket, &mut rx, &view);
        let (seen, outcome) = tokio::join!(daemon, session);

        assert_eq!(
            seen.unwrap(),
            vec![Request::Subscribe, Request::GetConfig, Request::ListDevices],
            "a reconnection must resynchronise state, config and devices, in that order"
        );
        assert!(outcome.is_ok());
        assert_eq!(*view.connected.lock().unwrap(), vec![true]);
    }

    #[tokio::test]
    async fn requests_queued_while_disconnected_are_dropped_not_replayed() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("s.sock");
        let listener = UnixListener::bind(&socket).unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel::<Request>();
        // Queued while offline: replaying this against a live alarm is exactly
        // the hazard the drain exists to prevent.
        tx.send(Request::Arm).unwrap();
        tx.send(Request::Toggle).unwrap();

        let view = RecordingView::default();
        let daemon = tokio::spawn(fake_daemon(listener, 3, vec![]));
        let session = run_session(&socket, &mut rx, &view);
        let (seen, _) = tokio::join!(daemon, session);

        let seen = seen.unwrap();
        assert!(
            !seen.contains(&Request::Arm) && !seen.contains(&Request::Toggle),
            "stale requests must never reach the daemon, saw: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_pushed_response_reaches_the_view() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("s.sock");
        let listener = UnixListener::bind(&socket).unwrap();

        let (_tx, mut rx) = mpsc::unbounded_channel::<Request>();
        let view = RecordingView::default();

        let daemon = tokio::spawn(fake_daemon(
            listener,
            3,
            vec![r#"{"event":"state_changed","state":"armed"}"#],
        ));
        let session = run_session(&socket, &mut rx, &view);
        let (_, outcome) = tokio::join!(daemon, session);

        assert!(outcome.unwrap().got_response, "a response must be recorded");
        assert_eq!(
            *view.applied.lock().unwrap(),
            vec![Response::StateChanged {
                state: GuardState::Armed
            }]
        );
    }
}
