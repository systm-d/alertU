//! AlertU command-line control.
//!
//! A thin, scriptable wrapper over the daemon socket: everything the tray and
//! the settings window can do, plus a `--json` mode that emits the raw
//! protocol responses so shell scripts can consume them.

mod render;

use alertu_common::config::Config;
use alertu_common::ipc_client::Client;
use alertu_common::protocol::{Response, DEFAULT_SOCKET_PATH};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use render::Outcome;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "alertu-ctl",
    version,
    about = "Control the AlertU daemon over its Unix socket"
)]
struct Cli {
    /// Path to the daemon's control socket.
    #[arg(long, short, global = true, default_value = DEFAULT_SOCKET_PATH)]
    socket: PathBuf,

    /// Emit the raw protocol response as JSON instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug, PartialEq)]
enum Command {
    /// Force-arm: lock the session and start watching.
    Arm,
    /// Force-disarm: unlock the session and stop watching.
    Disarm,
    /// Toggle arm/disarm, exactly as a remote button press would.
    Toggle,
    /// Print the current guard state.
    Status {
        /// Keep running, printing one line per state change.
        #[arg(long)]
        watch: bool,
    },
    /// Print the daemon's effective configuration.
    GetConfig,
    /// Replace the daemon's configuration from a TOML file (`-` reads stdin).
    SetConfig {
        /// Path to a TOML config, or `-` for stdin.
        file: PathBuf,
    },
    /// List the input devices the daemon can see.
    ListDevices,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("alertu-ctl: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.command {
        // Validate the file locally *before* connecting, so a malformed or
        // invalid config reports its own precise error instead of a generic
        // "is alertu-daemon running?" failure when there is nothing to blame
        // on the daemon at all.
        Command::SetConfig { file } => {
            let cfg = read_config(file)?;
            let mut client = Client::connect(&cli.socket)?;
            client.set_config(cfg)?;
            print_outcome(&Outcome::Ack, cli.json)
        }
        // `status --watch` streams instead of producing a single outcome, so
        // it gets its own arm and loop rather than joining the outcome match
        // below.
        Command::Status { watch: true } => {
            let mut client = Client::connect(&cli.socket)?;
            watch_states(&mut client, cli.json)
        }
        Command::Arm => with_client(cli, |client| {
            client.arm()?;
            Ok(Outcome::Ack)
        }),
        Command::Disarm => with_client(cli, |client| {
            client.disarm()?;
            Ok(Outcome::Ack)
        }),
        Command::Toggle => with_client(cli, |client| {
            client.toggle()?;
            Ok(Outcome::Ack)
        }),
        Command::Status { watch: false } => {
            with_client(cli, |client| Ok(Outcome::State(client.get_state()?)))
        }
        Command::GetConfig => with_client(cli, |client| {
            Ok(Outcome::Config(Box::new(client.get_config()?)))
        }),
        Command::ListDevices => {
            with_client(cli, |client| Ok(Outcome::Devices(client.list_devices()?)))
        }
    }
}

/// Connect, run a single request/response exchange, and print the result.
/// Shared by every command that needs nothing more than one round trip.
fn with_client(cli: &Cli, f: impl FnOnce(&mut Client) -> Result<Outcome>) -> Result<()> {
    let mut client = Client::connect(&cli.socket)?;
    let outcome = f(&mut client)?;
    print_outcome(&outcome, cli.json)
}

fn print_outcome(outcome: &Outcome, json: bool) -> Result<()> {
    println!("{}", render::render(outcome, json)?);
    Ok(())
}

/// The `status --watch` loop: print the current state, then one line per
/// push until the connection errors out or the daemon reports an error.
fn watch_states(client: &mut Client, json: bool) -> Result<()> {
    let state = client.subscribe()?;
    println!("{}", render::render(&Outcome::State(state), json)?);
    loop {
        match client.next_push()? {
            Response::State { state } | Response::StateChanged { state } => {
                println!("{}", render::render(&Outcome::State(state), json)?);
            }
            // Device-list pushes also arrive on a subscribed connection;
            // they are not state changes, so `status` ignores them.
            Response::Devices { .. } | Response::Config(_) | Response::Ok => {}
            Response::Error { message } => anyhow::bail!(message),
        }
    }
}

/// Read and validate a config locally, so a typo produces a precise error here
/// rather than a generic rejection after a round trip.
fn read_config(file: &PathBuf) -> Result<Config> {
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
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn socket_defaults_to_the_protocol_constant() {
        let cli = Cli::try_parse_from(["alertu-ctl", "status"]).unwrap();
        assert_eq!(cli.socket, PathBuf::from(DEFAULT_SOCKET_PATH));
        assert!(!cli.json);
    }

    #[test]
    fn global_flags_are_accepted_after_the_subcommand() {
        let cli =
            Cli::try_parse_from(["alertu-ctl", "status", "--json", "-s", "/tmp/a.sock"]).unwrap();
        assert!(cli.json);
        assert_eq!(cli.socket, PathBuf::from("/tmp/a.sock"));
    }

    #[test]
    fn status_watch_is_off_unless_asked() {
        let cli = Cli::try_parse_from(["alertu-ctl", "status"]).unwrap();
        assert_eq!(cli.command, Command::Status { watch: false });
        let cli = Cli::try_parse_from(["alertu-ctl", "status", "--watch"]).unwrap();
        assert_eq!(cli.command, Command::Status { watch: true });
    }

    #[test]
    fn set_config_takes_a_path_and_accepts_stdin() {
        let cli = Cli::try_parse_from(["alertu-ctl", "set-config", "/tmp/c.toml"]).unwrap();
        assert_eq!(
            cli.command,
            Command::SetConfig {
                file: PathBuf::from("/tmp/c.toml")
            }
        );
        let cli = Cli::try_parse_from(["alertu-ctl", "set-config", "-"]).unwrap();
        assert_eq!(
            cli.command,
            Command::SetConfig {
                file: PathBuf::from("-")
            }
        );
    }

    #[test]
    fn an_unknown_subcommand_is_a_usage_error() {
        let err = Cli::try_parse_from(["alertu-ctl", "explode"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn a_missing_subcommand_is_a_usage_error() {
        let err = Cli::try_parse_from(["alertu-ctl"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn set_config_without_a_file_is_a_usage_error() {
        let err = Cli::try_parse_from(["alertu-ctl", "set-config"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

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
