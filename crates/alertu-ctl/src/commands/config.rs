//! Reading and replacing the daemon's configuration.

use super::{print_outcome, with_client};
use crate::error::CliError;
use crate::render::Outcome;
use alertu_common::config::Config;
use alertu_common::ipc_client::Client;
use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;

/// Print the daemon's effective configuration.
pub fn get(socket: &Path, json: bool) -> Result<(), CliError> {
    with_client(socket, json, |client| {
        Ok(Outcome::Config(Box::new(client.get_config()?)))
    })
}

/// Replace the daemon's configuration from a TOML file (`-` reads stdin).
///
/// The file is validated locally *before* connecting, so a malformed or invalid
/// config reports its own precise error instead of a generic "is alertu-daemon
/// running?" failure when there is nothing to blame on the daemon at all.
pub fn set(socket: &Path, json: bool, file: &Path) -> Result<(), CliError> {
    let cfg = read_config(file).map_err(CliError::usage)?;
    let mut client = Client::connect(socket)?;
    client.set_config(cfg)?;
    print_outcome(&Outcome::Ack, json)?;
    Ok(())
}

/// Read and validate a config locally, so a typo produces a precise error here
/// rather than a generic rejection after a round trip.
pub(crate) fn read_config(file: &Path) -> Result<Config> {
    let text = if file.as_os_str() == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading config from stdin")?;
        buf
    } else {
        std::fs::read_to_string(file)
            .with_context(|| format!("reading config {}", file.display()))?
    };
    let cfg: Config = toml::from_str(&text).context("parsing config TOML")?;
    cfg.validate().context("validating config")?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_config_parses_a_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, toml::to_string_pretty(&Config::default()).unwrap()).unwrap();

        let cfg = read_config(&path).unwrap();

        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn read_config_rejects_empty_toggle_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "toggle_keys = []\n").unwrap();

        let err = read_config(&path).unwrap_err();

        assert!(
            err.chain().any(|c| c.to_string().contains("toggle_keys")),
            "got: {err:#}"
        );
    }

    #[test]
    fn read_config_names_the_path_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");

        let err = read_config(&path).unwrap_err();

        assert!(
            err.chain()
                .any(|c| c.to_string().contains(&*path.display().to_string())),
            "got: {err:#}"
        );
    }
}
