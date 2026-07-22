//! The command-line surface and its dispatch. No logic lives here.

use crate::commands;
use crate::error::CliError;
use alertu_common::protocol::DEFAULT_SOCKET_PATH;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "alertu-ctl",
    version,
    about = "Control the AlertU daemon over its Unix socket"
)]
pub struct Cli {
    /// Path to the daemon's control socket.
    #[arg(long, short, global = true, default_value = DEFAULT_SOCKET_PATH)]
    pub socket: PathBuf,

    /// Emit the raw protocol response as JSON instead of human-readable text.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Command {
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
    /// Write the default sound files (beep, warning tick, siren) into a directory.
    GenSounds {
        /// Destination directory, e.g. /usr/share/sounds/alertu.
        #[arg(long)]
        dir: PathBuf,
        /// Overwrite files that already exist.
        #[arg(long)]
        force: bool,
    },
}

/// Dispatch a parsed command. Every variant appears exactly once, so no
/// unreachable arm can exist and no command can be silently forgotten.
pub fn run(cli: &Cli) -> Result<(), CliError> {
    let socket = cli.socket.as_path();
    let json = cli.json;

    match &cli.command {
        Command::Arm => commands::state::arm(socket, json),
        Command::Disarm => commands::state::disarm(socket, json),
        Command::Toggle => commands::state::toggle(socket, json),
        Command::Status { watch: false } => commands::state::status(socket, json),
        Command::Status { watch: true } => commands::state::watch(socket, json),
        Command::GetConfig => commands::config::get(socket, json),
        Command::SetConfig { file } => commands::config::set(socket, json, file),
        Command::ListDevices => commands::devices::list(socket, json),
        // Never opens a socket, so it takes no socket path.
        Command::GenSounds { dir, force } => commands::gen_sounds::run(json, dir, *force),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{EXIT_DAEMON, EXIT_USAGE};
    use alertu_common::config::Config;
    use clap::CommandFactory;
    use std::path::Path;

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
    fn gen_sounds_parses_dir_and_force() {
        let cli = Cli::try_parse_from(["alertu-ctl", "gen-sounds", "--dir", "/tmp/s"]).unwrap();
        assert_eq!(
            cli.command,
            Command::GenSounds {
                dir: PathBuf::from("/tmp/s"),
                force: false
            }
        );
        let cli = Cli::try_parse_from(["alertu-ctl", "gen-sounds", "--dir", "/tmp/s", "--force"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Command::GenSounds { force: true, .. }
        ));
    }

    #[test]
    fn gen_sounds_requires_a_dir() {
        let err = Cli::try_parse_from(["alertu-ctl", "gen-sounds"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    /// A socket path that cannot exist, so no test ever reaches a real daemon.
    fn dead_socket(dir: &Path) -> PathBuf {
        dir.join("no-daemon-here.sock")
    }

    #[test]
    fn a_bad_config_file_exits_with_the_usage_code() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.toml");
        let cli = Cli::try_parse_from([
            "alertu-ctl",
            "-s",
            dead_socket(dir.path()).to_str().unwrap(),
            "set-config",
            missing.to_str().unwrap(),
        ])
        .unwrap();

        let err = run(&cli).unwrap_err();

        assert_eq!(err.code, EXIT_USAGE, "got: {:#}", err.source);
    }

    #[test]
    fn an_invalid_config_exits_with_the_usage_code() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "toggle_keys = []\n").unwrap();
        let cli = Cli::try_parse_from([
            "alertu-ctl",
            "-s",
            dead_socket(dir.path()).to_str().unwrap(),
            "set-config",
            path.to_str().unwrap(),
        ])
        .unwrap();

        let err = run(&cli).unwrap_err();

        assert_eq!(err.code, EXIT_USAGE, "got: {:#}", err.source);
    }

    #[test]
    fn a_config_that_reaches_a_dead_socket_exits_with_the_daemon_code() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, toml::to_string_pretty(&Config::default()).unwrap()).unwrap();
        let cli = Cli::try_parse_from([
            "alertu-ctl",
            "-s",
            dead_socket(dir.path()).to_str().unwrap(),
            "set-config",
            path.to_str().unwrap(),
        ])
        .unwrap();

        let err = run(&cli).unwrap_err();

        assert_eq!(err.code, EXIT_DAEMON, "got: {:#}", err.source);
    }

    #[test]
    fn an_unreachable_daemon_exits_with_the_daemon_code() {
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::try_parse_from([
            "alertu-ctl",
            "-s",
            dead_socket(dir.path()).to_str().unwrap(),
            "status",
        ])
        .unwrap();

        let err = run(&cli).unwrap_err();

        assert_eq!(err.code, EXIT_DAEMON, "got: {:#}", err.source);
    }

    /// `gen-sounds` never opens a socket, so exit 1 ("the daemon said no, try
    /// again later") is never the right answer for it: a script that retries on
    /// 1 would loop forever on a clobber refusal.
    #[test]
    fn gen_sounds_refusing_to_clobber_exits_with_the_usage_code() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("beep.wav"), b"precious").unwrap();
        let cli = Cli::try_parse_from([
            "alertu-ctl",
            "gen-sounds",
            "--dir",
            dir.path().to_str().unwrap(),
        ])
        .unwrap();

        let err = run(&cli).unwrap_err();

        assert_eq!(err.code, EXIT_USAGE, "got: {:#}", err.source);
    }

    #[test]
    fn gen_sounds_into_an_impossible_dir_exits_with_the_usage_code() {
        let cli = Cli::try_parse_from(["alertu-ctl", "gen-sounds", "--dir", "/proc/nope/sounds"])
            .unwrap();

        let err = run(&cli).unwrap_err();

        assert_eq!(err.code, EXIT_USAGE, "got: {:#}", err.source);
    }
}
