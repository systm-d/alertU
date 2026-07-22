//! Blocking request/response client to the daemon socket.
//!
//! The settings window is a plain synchronous program — it does not subscribe
//! to pushes, so each request has exactly one matching reply on the same
//! connection, which keeps this dead simple (no async runtime needed).

use crate::config::Config;
use crate::protocol::{InputDeviceInfo, Request, Response};
use crate::state::GuardState;
use anyhow::{Context, Result, anyhow};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Whether replaying `req` after a dropped connection is safe.
///
/// A dropped connection is no evidence about whether the daemon already acted,
/// so only requests that are idempotent *and* carry no session state may be
/// replayed. `Subscribe` is excluded because a reopened connection is no longer
/// subscribed, and `next_push` would then block forever.
///
/// Matched exhaustively on purpose: adding a request to the protocol should
/// force a decision here rather than silently defaulting either way.
fn is_replay_safe(req: &Request) -> bool {
    match req {
        Request::GetConfig | Request::ListDevices | Request::GetState | Request::SetConfig(_) => {
            true
        }
        Request::Arm | Request::Disarm | Request::Toggle | Request::Subscribe => false,
    }
}

/// The parenthetical after a failed connect: which hypothesis to check first.
///
/// `EACCES` is not "the daemon is down". The socket is `0660` and group-owned,
/// so a permission error means the daemon is running and this login is simply
/// not in its group — by far the commonest failure for anyone upgrading from a
/// world-connectable socket. Sending them to look for a dead daemon wastes the
/// one diagnostic they get, so name the actual fix instead.
fn connect_hint(kind: std::io::ErrorKind) -> &'static str {
    if kind == std::io::ErrorKind::PermissionDenied {
        "permission denied: your login is probably not in the socket's group — \
         run `sudo usermod -aG alertu \"$USER\"`, then start a new login session"
    } else {
        "is alertu-daemon running?"
    }
}

/// A live connection to `alertu-daemon`.
pub struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    /// Kept so the connection can be re-established on demand.
    socket: PathBuf,
    reconnect: bool,
}

impl Client {
    /// Connect to the daemon's Unix socket.
    pub fn connect(socket: &Path) -> Result<Client> {
        let writer = UnixStream::connect(socket).map_err(|e| {
            let hint = connect_hint(e.kind());
            anyhow::Error::new(e).context(format!("connecting to {} ({hint})", socket.display()))
        })?;
        let reader = BufReader::new(writer.try_clone().context("cloning socket")?);
        Ok(Client {
            writer,
            reader,
            socket: socket.to_path_buf(),
            reconnect: false,
        })
    }

    /// Reconnect and retry once when a replay-safe request fails.
    ///
    /// Opt-in, and deliberately not the default: reconnecting silently hides a
    /// daemon restart, which not every caller wants. It is *not* what makes the
    /// retry safe, though — that is [`is_replay_safe`], which is consulted per
    /// request, so enabling this can never cause an `Arm` to be replayed.
    ///
    /// The only caller today is the settings window, which sends nothing but
    /// `GetConfig`, `ListDevices` and `SetConfig`.
    pub fn with_reconnect(mut self) -> Self {
        self.reconnect = true;
        self
    }

    /// Re-establish the connection in place, keeping the same `Client`.
    fn reopen(&mut self) -> Result<()> {
        let fresh = Client::connect(&self.socket)?;
        self.writer = fresh.writer;
        self.reader = fresh.reader;
        Ok(())
    }

    /// Send one request and read exactly one reply, retrying once through a
    /// fresh connection when [`Client::with_reconnect`] is enabled *and* the
    /// request is replay-safe.
    ///
    /// Any failure of a replay-safe request may be retried, I/O or not: a
    /// mangled reply is no more evidence of what the daemon did than a dropped
    /// connection is, and re-asking a question with no side effects costs
    /// nothing.
    fn round_trip(&mut self, req: &Request) -> Result<Response> {
        match self.attempt_round_trip(req) {
            Ok(resp) => Ok(resp),
            Err(e) if self.reconnect && is_replay_safe(req) => {
                self.reopen().context(format!("retrying after: {e}"))?;
                // Name both failures: without the first one, the settings
                // window would report a bare "daemon closed the connection"
                // with no hint that a reconnection had even been attempted.
                // `{e:#}` and not `{e}`: the interesting part of the first
                // failure is usually its cause ("reading reply: daemon closed
                // the connection"), which the plain Display would drop.
                self.attempt_round_trip(req).with_context(|| {
                    format!("failed again after reconnecting (first failure: {e:#})")
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Attempt one request/response round trip without retry.
    fn attempt_round_trip(&mut self, req: &Request) -> Result<Response> {
        let mut line = serde_json::to_string(req).context("serializing request")?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .context("writing request")?;
        self.writer.flush().context("flushing request")?;
        self.read_response()
    }

    /// Read a single newline-delimited response off the socket.
    fn read_response(&mut self) -> Result<Response> {
        let mut buf = String::new();
        let n = self.reader.read_line(&mut buf).context("reading reply")?;
        if n == 0 {
            return Err(anyhow!("daemon closed the connection"));
        }
        serde_json::from_str(buf.trim_end()).context("parsing reply")
    }

    pub fn get_config(&mut self) -> Result<Config> {
        match self.round_trip(&Request::GetConfig)? {
            Response::Config(cfg) => Ok(*cfg),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(anyhow!("unexpected reply to GetConfig: {other:?}")),
        }
    }

    pub fn list_devices(&mut self) -> Result<Vec<InputDeviceInfo>> {
        match self.round_trip(&Request::ListDevices)? {
            Response::Devices { devices } => Ok(devices),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(anyhow!("unexpected reply to ListDevices: {other:?}")),
        }
    }

    pub fn get_state(&mut self) -> Result<GuardState> {
        match self.round_trip(&Request::GetState)? {
            Response::State { state } => Ok(state),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(anyhow!("unexpected reply to GetState: {other:?}")),
        }
    }

    pub fn set_config(&mut self, cfg: Config) -> Result<()> {
        match self.round_trip(&Request::SetConfig(Box::new(cfg)))? {
            Response::Ok => Ok(()),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(anyhow!("unexpected reply to SetConfig: {other:?}")),
        }
    }

    /// Force-arm, locking the session.
    pub fn arm(&mut self) -> Result<()> {
        self.expect_ok(&Request::Arm)
    }

    /// Force-disarm, unlocking the session.
    pub fn disarm(&mut self) -> Result<()> {
        self.expect_ok(&Request::Disarm)
    }

    /// Toggle arm/disarm, exactly as a remote button press would.
    pub fn toggle(&mut self) -> Result<()> {
        self.expect_ok(&Request::Toggle)
    }

    /// Register for asynchronous pushes; returns the state snapshot the daemon
    /// sends in reply. Follow with [`Client::next_push`].
    pub fn subscribe(&mut self) -> Result<GuardState> {
        match self.round_trip(&Request::Subscribe)? {
            Response::State { state } => Ok(state),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(anyhow!("unexpected reply to Subscribe: {other:?}")),
        }
    }

    /// Block until the daemon pushes the next response. Only meaningful after
    /// [`Client::subscribe`]; on a non-subscribed connection this blocks forever.
    pub fn next_push(&mut self) -> Result<Response> {
        self.read_response()
    }

    /// Round-trip a request whose only successful answer is a bare `Ok`.
    fn expect_ok(&mut self, req: &Request) -> Result<()> {
        match self.round_trip(req)? {
            Response::Ok => Ok(()),
            Response::Error { message } => Err(anyhow!(message)),
            other => Err(anyhow!("unexpected reply to {req:?}: {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread;

    /// Start a fake daemon in a background thread: it accepts one connection,
    /// reads a single request line, then writes every reply in `replies`.
    /// Returns the socket path.
    fn fake_daemon(dir: &std::path::Path, replies: &'static [&'static str]) -> PathBuf {
        let path = dir.join("fake.sock");
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            for reply in replies {
                writeln!(stream, "{reply}").unwrap();
            }
            stream.flush().unwrap();
        });
        path
    }

    /// A fake daemon that serves several connections in turn. Each entry is one
    /// session's replies; an empty slice means "take the request, then hang up
    /// without answering", which is what a daemon restart looks like to a
    /// client mid-request.
    ///
    /// The request is consumed even when there are no replies, so the client's
    /// write always lands and its read always sees a clean EOF. Closing before
    /// the write would race, surfacing `Connection reset by peer` instead —
    /// same outcome, different message, flaky assertions.
    fn fake_daemon_sessions(
        dir: &std::path::Path,
        sessions: &'static [&'static [&'static str]],
    ) -> PathBuf {
        let path = dir.join("multi.sock");
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            for replies in sessions {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                if replies.is_empty() {
                    continue; // drop the connection unanswered
                }
                for reply in *replies {
                    let _ = writeln!(stream, "{reply}");
                }
                let _ = stream.flush();
            }
        });
        path
    }

    /// The socket is `0660`, so "I can't connect" now usually means "I'm not in
    /// the group", not "the daemon is down". The error has to say so: this is
    /// the only diagnostic an upgrading user gets.
    ///
    /// Not guarded by a uid check (this crate is `#![forbid(unsafe_code)]`, so
    /// no `geteuid`) but by the outcome: root ignores the mode, the connect
    /// succeeds, and there is no permission error to assert on — skip rather
    /// than assert something vacuous.
    #[test]
    fn a_permission_denied_connect_names_the_group() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.sock");
        let _listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let Err(err) = Client::connect(&path) else {
            return; // euid 0, or a filesystem that does not enforce the mode
        };

        let text = format!("{err:#}");
        assert!(
            text.contains("not in the socket's group"),
            "a permission error must blame the group, got: {text}"
        );
        assert!(
            text.contains("usermod -aG alertu"),
            "a permission error must name the fix, got: {text}"
        );
        assert!(
            !text.contains("is alertu-daemon running?"),
            "the daemon is running; that hypothesis only misleads here: {text}"
        );
    }

    #[test]
    fn arm_accepts_an_ok_reply() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon(dir.path(), &[r#"{"event":"ok"}"#]);
        let mut client = Client::connect(&path).unwrap();
        client.arm().unwrap();
    }

    #[test]
    fn an_error_reply_becomes_an_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon(dir.path(), &[r#"{"event":"error","message":"nope"}"#]);
        let mut client = Client::connect(&path).unwrap();
        let err = client.toggle().unwrap_err();
        assert!(err.to_string().contains("nope"), "got: {err}");
    }

    #[test]
    fn an_unexpected_reply_becomes_an_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon(dir.path(), &[r#"{"event":"state","state":"idle"}"#]);
        let mut client = Client::connect(&path).unwrap();
        assert!(client.disarm().is_err());
    }

    #[test]
    fn subscribe_returns_the_snapshot_then_streams_pushes() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon(
            dir.path(),
            &[
                r#"{"event":"state","state":"idle"}"#,
                r#"{"event":"state_changed","state":"armed"}"#,
            ],
        );
        let mut client = Client::connect(&path).unwrap();
        assert_eq!(client.subscribe().unwrap(), GuardState::Idle);
        assert_eq!(
            client.next_push().unwrap(),
            Response::StateChanged {
                state: GuardState::Armed
            }
        );
    }

    #[test]
    fn without_reconnect_a_dropped_connection_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon_sessions(dir.path(), &[&[], &[r#"{"event":"ok"}"#]]);
        let mut client = Client::connect(&path).unwrap();
        assert!(
            client.set_config(Config::default()).is_err(),
            "a client without reconnect must surface the dropped connection"
        );
    }

    #[test]
    fn with_reconnect_a_dropped_connection_is_retried_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon_sessions(dir.path(), &[&[], &[r#"{"event":"ok"}"#]]);
        let mut client = Client::connect(&path).unwrap().with_reconnect();
        client
            .set_config(Config::default())
            .expect("the retry should have reached the second session");
    }

    /// FIX 3's wording, pinned: when the retry itself fails, the surfaced error
    /// must still name the original failure, or the settings window reports a
    /// bare "daemon closed the connection" with no sign a retry happened.
    #[test]
    fn with_reconnect_a_failed_retry_names_both_failures() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_daemon_sessions(dir.path(), &[&[], &[]]);
        let mut client = Client::connect(&path).unwrap().with_reconnect();

        let err = client.set_config(Config::default()).unwrap_err();

        let text = format!("{err:#}");
        assert!(
            text.contains("failed again after reconnecting"),
            "the error should say a retry was attempted, got: {text}"
        );
        assert!(
            text.contains("daemon closed the connection"),
            "the error should carry the original failure, got: {text}"
        );
    }

    /// The safety of the retry lives in `is_replay_safe`, not in a doc comment:
    /// enabling reconnection must not make a state-changing request replayable.
    #[test]
    fn replay_safety_is_decided_per_request() {
        for req in [
            Request::GetConfig,
            Request::ListDevices,
            Request::GetState,
            Request::SetConfig(Box::default()),
        ] {
            assert!(is_replay_safe(&req), "{req:?} should be replay-safe");
        }
        for req in [
            Request::Arm,
            Request::Disarm,
            Request::Toggle,
            // A reopened connection is not subscribed; retrying would leave
            // `next_push` blocking forever on a connection nobody feeds.
            Request::Subscribe,
        ] {
            assert!(!is_replay_safe(&req), "{req:?} must never be replayed");
        }
    }

    /// A first session that hangs up unanswered, then a second one standing by
    /// with an answer. Anything the client retries therefore *succeeds* here —
    /// which is what gives the assertions below teeth: a request that must not
    /// be replayed can only fail if the gate actually stopped it.
    const DROP_THEN_OK: &[&[&str]] = &[&[], &[r#"{"event":"ok"}"#]];
    const DROP_THEN_STATE: &[&[&str]] = &[&[], &[r#"{"event":"state","state":"idle"}"#]];

    fn reconnecting_client(
        dir: &std::path::Path,
        sessions: &'static [&'static [&'static str]],
    ) -> Client {
        let path = fake_daemon_sessions(dir, sessions);
        Client::connect(&path).unwrap().with_reconnect()
    }

    #[test]
    fn with_reconnect_arm_is_never_retried() {
        let dir = tempfile::tempdir().unwrap();
        let mut client = reconnecting_client(dir.path(), DROP_THEN_OK);
        assert!(
            client.arm().is_err(),
            "a replayed Arm could re-arm an alarm the user just disarmed"
        );
    }

    #[test]
    fn with_reconnect_disarm_is_never_retried() {
        let dir = tempfile::tempdir().unwrap();
        let mut client = reconnecting_client(dir.path(), DROP_THEN_OK);
        assert!(client.disarm().is_err(), "Disarm must not be replayed");
    }

    #[test]
    fn with_reconnect_toggle_is_never_retried() {
        let dir = tempfile::tempdir().unwrap();
        let mut client = reconnecting_client(dir.path(), DROP_THEN_OK);
        assert!(
            client.toggle().is_err(),
            "a replayed Toggle would land on the opposite state"
        );
    }

    #[test]
    fn with_reconnect_subscribe_is_never_retried() {
        let dir = tempfile::tempdir().unwrap();
        let mut client = reconnecting_client(dir.path(), DROP_THEN_STATE);
        assert!(
            client.subscribe().is_err(),
            "a silently reopened connection is not subscribed, and next_push \
             (which has no retry of its own) would block on it forever"
        );
    }
}
