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
}
