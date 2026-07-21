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

/// Why a D-Bus subscription in [`dbus_watch`] stopped.
enum Stopped {
    /// `SessionCtl`'s id no longer matches the one this subscription was
    /// built for (a live reconfigure via `SetConfig`). The caller should
    /// re-subscribe to the new session rather than fall back to polling.
    SessionChanged,
    /// The property stream ended on its own (bus or session gone).
    StreamEnded,
    /// The receiving end of the lock-state channel is gone: the state machine
    /// is shutting down, so there is nobody left to observe *for*. Distinct
    /// from `StreamEnded` because falling back to polling here would spawn
    /// `loginctl` twice a second for a receiver that no longer exists.
    MachineGone,
}

/// How often [`dbus_watch`] re-checks whether the session id it subscribed
/// with is still current. Reconfiguration is a rare, operator-driven event,
/// not a hot path, so this only needs to catch up eventually.
const RECHECK_INTERVAL: Duration = Duration::from_secs(5);

/// Observe the session's lock state, preferring D-Bus and falling back to
/// polling.
///
/// A precision that shapes this: logind's `Lock`/`Unlock` signals are *requests*
/// addressed to the screen locker, not notifications that anything changed. The
/// observable state change is `PropertiesChanged` on `LockedHint`, which is the
/// real-time equivalent of what `monitor` polls.
///
/// `session`'s id can change underneath this call (`SessionCtl::update_from`,
/// driven by `SetConfig`), so a live D-Bus subscription is torn down and
/// re-established whenever that happens rather than being left pointed at a
/// stale session. Any other failure — no system bus, an unknown session, a
/// dropped stream — falls back to the poll loop rather than leaving the
/// daemon blind, because unlock detection is what disarms the alarm after a
/// password unlock.
pub async fn watch(session: SessionCtl, tx: mpsc::Sender<bool>, poll_interval: Duration) {
    loop {
        match dbus_watch(&session, &tx).await {
            Ok(Stopped::SessionChanged) => {
                debug!("session id changed; re-subscribing over D-Bus");
                continue;
            }
            Ok(Stopped::StreamEnded) => {
                warn!("logind property stream ended; falling back to polling");
                break;
            }
            Ok(Stopped::MachineGone) => {
                // Not a fallback case: polling would spawn `loginctl` every
                // `poll_interval` forever with nobody reading the results.
                debug!("lock-state receiver dropped; stopping the session watch");
                return;
            }
            Err(e) => {
                warn!(error = %e, "cannot observe logind over D-Bus; falling back to polling");
                break;
            }
        }
    }
    monitor(session, tx, poll_interval).await;
}

/// Subscribe to `LockedHint` changes on `session`'s current session.
///
/// Races the property stream against a periodic re-check of `session.id()`
/// so a live reconfigure (see [`watch`]) is noticed even though the D-Bus
/// subscription itself is bound to whatever session id was current when it
/// was established.
async fn dbus_watch(session: &SessionCtl, tx: &mpsc::Sender<bool>) -> anyhow::Result<Stopped> {
    use futures_util::StreamExt;

    let session_id = session.id();
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
    let path: zbus::zvariant::OwnedObjectPath =
        manager.call("GetSession", &(session_id.as_str(),)).await?;
    debug!(session = %session_id, path = %path.as_str(), "watching logind session over D-Bus");

    let props = zbus::fdo::PropertiesProxy::builder(&conn)
        .destination("org.freedesktop.login1")?
        .path(path)?
        .build()
        .await?;

    let wanted = zbus::names::InterfaceName::try_from("org.freedesktop.login1.Session")?;
    let mut changes = props.receive_properties_changed().await?;

    let mut recheck = tokio::time::interval(RECHECK_INTERVAL);
    recheck.tick().await; // the first tick fires immediately; skip it

    loop {
        tokio::select! {
            signal = changes.next() => {
                let Some(signal) = signal else {
                    return Ok(Stopped::StreamEnded);
                };
                let args = signal.args()?;
                if args.interface_name != wanted {
                    continue;
                }
                if let Some(value) = args.changed_properties.get("LockedHint") {
                    match bool::try_from(value.try_clone()?) {
                        Ok(locked) => {
                            debug!(locked, "logind reported a lock-state change");
                            if tx.send(locked).await.is_err() {
                                return Ok(Stopped::MachineGone);
                            }
                        }
                        Err(e) => warn!(error = %e, "LockedHint was not a boolean"),
                    }
                }
            }
            _ = recheck.tick() => {
                if session_changed(&session_id, session) {
                    return Ok(Stopped::SessionChanged);
                }
            }
        }
    }
}

/// True when `session`'s current id no longer matches `subscribed_id`.
///
/// This is the pure comparison behind `dbus_watch`'s periodic re-check;
/// pulled out on its own so it can be unit-tested without a live D-Bus
/// session.
fn session_changed(subscribed_id: &str, session: &SessionCtl) -> bool {
    session.id() != subscribed_id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl(id: &str) -> SessionCtl {
        SessionCtl {
            id: Arc::new(Mutex::new(id.to_string())),
        }
    }

    #[test]
    fn session_changed_is_false_while_the_id_is_unchanged() {
        let session = ctl("3");
        assert!(!session_changed("3", &session));
    }

    #[test]
    fn session_changed_is_true_after_a_live_reconfigure() {
        let session = ctl("3");
        *session.id.lock().unwrap() = "7".to_string();
        assert!(session_changed("3", &session));
        assert!(!session_changed("7", &session));
    }
}
