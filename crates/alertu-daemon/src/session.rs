//! logind session control and lock-state monitoring.
//!
//! Actions (`lock`, `unlock`) always shell out to `loginctl`: they are rare, and
//! staying on the CLI keeps the black-box tests' `PATH` shim in control of them.
//!
//! Lock-state *observation* prefers a D-Bus subscription to `PropertiesChanged`
//! on `LockedHint`, falling back to polling `loginctl` when the bus or the
//! session is unavailable — see [`watch`].

use alertu_common::config::Config;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Controls a single logind session (lock/unlock, query lock state).
///
/// The session id lives behind a shared lock so it can be swapped when the
/// config changes without recreating the monitor task.
#[derive(Clone)]
pub struct SessionCtl {
    id: Arc<Mutex<String>>,
}

impl SessionCtl {
    /// Build a controller, resolving the session id from config
    /// (`"auto"` → the active session on `seat0`, falling back to the first
    /// listed session).
    pub async fn new(cfg: &Config) -> Self {
        let id = resolve_session_id(cfg).await;
        SessionCtl {
            id: Arc::new(Mutex::new(id)),
        }
    }

    /// The current session id.
    pub fn id(&self) -> String {
        self.id.lock().unwrap().clone()
    }

    /// Re-point at the session implied by `cfg` (used on live reconfigure).
    pub async fn update_from(&self, cfg: &Config) {
        let id = resolve_session_id(cfg).await;
        *self.id.lock().unwrap() = id;
    }

    /// Lock the session (`loginctl lock-session`).
    pub async fn lock(&self) {
        self.run("lock-session").await;
    }

    /// Unlock the session (`loginctl unlock-session`).
    pub async fn unlock(&self) {
        self.run("unlock-session").await;
    }

    async fn run(&self, verb: &str) {
        let id = self.id();
        if id.is_empty() {
            warn!(
                action = verb,
                "no session id resolved; cannot control session"
            );
            return;
        }
        match Command::new("loginctl").arg(verb).arg(&id).output().await {
            Ok(out) if out.status.success() => debug!(action = verb, session = %id, "ok"),
            Ok(out) => warn!(
                action = verb,
                session = %id,
                stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                "loginctl failed"
            ),
            Err(e) => warn!(action = verb, error = %e, "failed to run loginctl"),
        }
    }

    /// Query `LockedHint`. Returns `None` if it could not be read.
    pub async fn is_locked(&self) -> Option<bool> {
        let id = self.id();
        if id.is_empty() {
            return None;
        }
        let out = Command::new("loginctl")
            .args(["show-session", &id, "-p", "LockedHint", "--value"])
            .output()
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let value = String::from_utf8_lossy(&out.stdout);
        match value.trim() {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        }
    }
}

/// Resolve a concrete session id from config.
async fn resolve_session_id(cfg: &Config) -> String {
    if !cfg.session_is_auto() {
        return cfg.session_id.clone();
    }

    // Preferred: the active session on the primary seat.
    if let Ok(out) = Command::new("loginctl")
        .args(["show-seat", "seat0", "-p", "ActiveSession", "--value"])
        .output()
        .await
    {
        if out.status.success() {
            let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !id.is_empty() {
                debug!(session = %id, "resolved active session on seat0");
                return id;
            }
        }
    }

    // Fallback: first session from `loginctl list-sessions`.
    if let Ok(out) = Command::new("loginctl")
        .args(["list-sessions", "--no-legend"])
        .output()
        .await
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(first) = text.lines().next() {
                if let Some(id) = first.split_whitespace().next() {
                    debug!(session = %id, "resolved first listed session");
                    return id.to_string();
                }
            }
        }
    }

    warn!("could not resolve any logind session id");
    String::new()
}

/// Poll `LockedHint` and push a boolean whenever it changes.
///
/// The initial observed value is not emitted, so the machine only reacts to
/// genuine transitions (e.g. a password unlock while armed).
pub async fn monitor(session: SessionCtl, tx: mpsc::Sender<bool>, interval: Duration) {
    let mut last: Option<bool> = None;
    loop {
        tokio::time::sleep(interval).await;
        if let Some(locked) = session.is_locked().await {
            if last != Some(locked) {
                if last.is_some() {
                    debug!(locked, "session lock state changed");
                    if tx.send(locked).await.is_err() {
                        return;
                    }
                }
                last = Some(locked);
            }
        }
    }
}

/// Observe the session's lock state, preferring D-Bus and falling back to
/// polling.
///
/// A precision that shapes this: logind's `Lock`/`Unlock` signals are *requests*
/// addressed to the screen locker, not notifications that anything changed. The
/// observable state change is `PropertiesChanged` on `LockedHint`, which is the
/// real-time equivalent of what `monitor` polls.
///
/// Any failure — no system bus, an unknown session, a dropped stream — falls
/// back to the poll loop rather than leaving the daemon blind, because unlock
/// detection is what disarms the alarm after a password unlock.
pub async fn watch(session: SessionCtl, tx: mpsc::Sender<bool>, poll_interval: Duration) {
    match dbus_watch(&session.id(), &tx).await {
        Ok(()) => warn!("logind property stream ended; falling back to polling"),
        Err(e) => warn!(error = %e, "cannot observe logind over D-Bus; falling back to polling"),
    }
    monitor(session, tx, poll_interval).await;
}

/// Subscribe to `LockedHint` changes. Returns `Ok(())` only when the stream ends.
async fn dbus_watch(session_id: &str, tx: &mpsc::Sender<bool>) -> anyhow::Result<()> {
    use futures_util::StreamExt;

    if session_id.is_empty() {
        anyhow::bail!("no session id resolved");
    }

    let conn = zbus::Connection::system().await?;

    let manager = zbus::Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;
    let path: zbus::zvariant::OwnedObjectPath = manager.call("GetSession", &(session_id,)).await?;
    debug!(session = %session_id, path = %path.as_str(), "watching logind session over D-Bus");

    let props = zbus::fdo::PropertiesProxy::builder(&conn)
        .destination("org.freedesktop.login1")?
        .path(path)?
        .build()
        .await?;

    let wanted = zbus::names::InterfaceName::try_from("org.freedesktop.login1.Session")?;
    let mut changes = props.receive_properties_changed().await?;

    while let Some(signal) = changes.next().await {
        let args = signal.args()?;
        if args.interface_name != wanted {
            continue;
        }
        if let Some(value) = args.changed_properties.get("LockedHint") {
            match bool::try_from(value.try_clone()?) {
                Ok(locked) => {
                    debug!(locked, "logind reported a lock-state change");
                    if tx.send(locked).await.is_err() {
                        return Ok(()); // the machine shut down
                    }
                }
                Err(e) => warn!(error = %e, "LockedHint was not a boolean"),
            }
        }
    }
    Ok(())
}
