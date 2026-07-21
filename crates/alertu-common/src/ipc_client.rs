//! Blocking request/response client to the daemon socket.
//!
//! The settings window is a plain synchronous program — it does not subscribe
//! to pushes, so each request has exactly one matching reply on the same
//! connection, which keeps this dead simple (no async runtime needed).

use crate::config::Config;
use crate::protocol::{InputDeviceInfo, Request, Response};
use crate::state::GuardState;
use anyhow::{anyhow, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

/// A live connection to `alertu-daemon`.
pub struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    /// Connect to the daemon's Unix socket.
    pub fn connect(socket: &Path) -> Result<Client> {
        let writer = UnixStream::connect(socket).with_context(|| {
            format!(
                "connecting to {} (is alertu-daemon running?)",
                socket.display()
            )
        })?;
        let reader = BufReader::new(writer.try_clone().context("cloning socket")?);
        Ok(Client { writer, reader })
    }

    /// Send one request and read exactly one reply.
    fn round_trip(&mut self, req: &Request) -> Result<Response> {
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
}
