//! AlertU command-line control.
//!
//! A thin, scriptable wrapper over the daemon socket: everything the tray and
//! the settings window can do, plus a `--json` mode that emits the raw
//! protocol responses so shell scripts can consume them.

#![forbid(unsafe_code)]

mod render;
mod sounds;

use alertu_common::config::Config;
use alertu_common::ipc_client::Client;
use alertu_common::protocol::{DEFAULT_SOCKET_PATH, Response};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use render::Outcome;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Something went wrong on the daemon side: it refused the request, or there
/// was nothing listening on the socket at all.
const EXIT_DAEMON: u8 = 1;
/// Bad input from the caller, caught locally before any socket is touched.
/// Same code clap uses for a malformed command line.
const EXIT_USAGE: u8 = 2;

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

/// A failure together with the exit code it maps to, so the shell can tell
/// "the daemon said no" (1) from "you asked for something impossible" (2).
#[derive(Debug)]
struct CliError {
    code: u8,
    source: anyhow::Error,
}

impl CliError {
    /// Bad local input: exits 2, like clap's own usage errors.
    fn usage(source: anyhow::Error) -> CliError {
        CliError {
            code: EXIT_USAGE,
            source,
        }
    }
}

/// Anything propagated with `?` from a socket exchange is a daemon or
/// connection failure; usage errors are tagged explicitly at their source.
impl From<anyhow::Error> for CliError {
    fn from(source: anyhow::Error) -> CliError {
        CliError {
            code: EXIT_DAEMON,
            source,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("alertu-ctl: {:#}", e.source);
            ExitCode::from(e.code)
        }
    }
}

/// The arm/disarm chirp and the alarm siren, embedded so `gen-sounds` needs no
/// data files at runtime. Both are recordings rather than synthesis: they simply
/// sound better than a sine sweep. Only the countdown tick is still generated,
/// because it has to be a few tens of milliseconds — `play_once` spawns a player
/// once a second, so anything longer would pile up.
///
/// They live under `assets/` rather than the repository's `resources/` because
/// `cargo package` only includes files inside the crate; reaching outside it
/// would make this crate unpublishable.
const BEEP_WAV: &[u8] = include_bytes!("../assets/lock.wav");
const SIREN_WAV: &[u8] = include_bytes!("../assets/siren.wav");

/// Modes forced onto the destination directory and the files written into it.
///
/// Explicit, because the documented install step is
/// `sudo alertu-ctl gen-sounds --dir /usr/share/sounds/alertu`: with a umask of
/// `0027` the defaults would land as root-owned 0750/0640, which the
/// unprivileged `alertu` service account cannot read. That failure is silent —
/// `sound.rs::play_once` only checks `path.exists()` and sends the player's
/// stderr to `/dev/null` — so all audio would simply disappear.
const DIR_MODE: u32 = 0o755;
const FILE_MODE: u32 = 0o644;

/// Write the three default sounds into `dir`.
///
/// Refuses to overwrite unless `force`, because the natural destination is a
/// system directory a user may have customised.
fn write_sounds(dir: &Path, force: bool) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    // Applied whether or not we created it: the whole point of this command is
    // to leave behind sounds the daemon's service account can actually read.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(DIR_MODE))
        .with_context(|| format!("setting mode on {}", dir.display()))?;

    let files: [(&str, Vec<u8>); 3] = [
        ("beep.wav", BEEP_WAV.to_vec()),
        ("warning.wav", sounds::encode_wav(&sounds::warning_tick())),
        ("siren.wav", SIREN_WAV.to_vec()),
    ];

    // Check every destination before writing any of them, so a refusal
    // never leaves the directory half-populated: either nothing was written,
    // or all three were.
    if !force {
        for (name, _) in &files {
            let path = dir.join(name);
            if path.exists() {
                anyhow::bail!(
                    "{} already exists; pass --force to overwrite",
                    path.display()
                );
            }
        }
    }

    for (name, bytes) in &files {
        let path = dir.join(name);
        std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        // After the write, so an overwritten file's old, tighter mode is
        // corrected too — `fs::write` keeps the mode of a file that exists.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(FILE_MODE))
            .with_context(|| format!("setting mode on {}", path.display()))?;
    }
    Ok(())
}

fn run(cli: &Cli) -> Result<(), CliError> {
    match &cli.command {
        // `gen-sounds` never opens a socket, so every way it can fail is a
        // local input error: exit 2, not the daemon code. It lives in this
        // match rather than behind an early return precisely so that no
        // second, unreachable `GenSounds` arm can exist.
        Command::GenSounds { dir, force } => {
            write_sounds(dir, *force).map_err(CliError::usage)?;
            print_outcome(&Outcome::Ack, cli.json)?;
            Ok(())
        }
        // Validate the file locally *before* connecting, so a malformed or
        // invalid config reports its own precise error instead of a generic
        // "is alertu-daemon running?" failure when there is nothing to blame
        // on the daemon at all.
        Command::SetConfig { file } => {
            let cfg = read_config(file).map_err(CliError::usage)?;
            let mut client = Client::connect(&cli.socket)?;
            client.set_config(cfg)?;
            print_outcome(&Outcome::Ack, cli.json)?;
            Ok(())
        }
        // `status --watch` streams instead of producing a single outcome, so
        // it gets its own arm and loop rather than joining the outcome match
        // below.
        Command::Status { watch: true } => {
            let mut client = Client::connect(&cli.socket)?;
            watch_states(&mut client, cli.json)?;
            Ok(())
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
fn with_client(cli: &Cli, f: impl FnOnce(&mut Client) -> Result<Outcome>) -> Result<(), CliError> {
    let mut client = Client::connect(&cli.socket)?;
    let outcome = f(&mut client)?;
    print_outcome(&outcome, cli.json)?;
    Ok(())
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
        let push = client.next_push()?;
        match push {
            // `--json` is contractually the raw `Response`, so a push keeps its
            // own `state_changed` tag: a consumer must be able to tell the
            // initial snapshot from a later transition. The human form is just
            // the bare state label either way.
            Response::State { state } | Response::StateChanged { state } => {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string(&push).context("serializing JSON output")?
                    );
                } else {
                    println!("{}", render::render(&Outcome::State(state), false)?);
                }
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
fn read_config(file: &Path) -> Result<Config> {
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

    #[test]
    fn gen_sounds_writes_three_playable_files() {
        let dir = tempfile::tempdir().unwrap();
        write_sounds(dir.path(), false).unwrap();
        for name in ["beep.wav", "warning.wav", "siren.wav"] {
            let bytes = std::fs::read(dir.path().join(name)).unwrap();
            assert_eq!(&bytes[0..4], b"RIFF", "{name} is not a RIFF file");
            assert_eq!(&bytes[8..12], b"WAVE", "{name} is not a WAVE file");
        }
    }

    #[test]
    fn gen_sounds_refuses_to_clobber_without_force() {
        // warning.wav sits in the middle of the write order (beep, warning,
        // siren): a check-then-write loop that processes one file at a time
        // would have already written beep.wav by the time it hits this
        // conflict, and would never reach siren.wav. Placing the pre-existing
        // file here is what actually catches that bug — a conflict on the
        // *first* file wouldn't, since nothing would have been written yet
        // either way.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("warning.wav"), b"precious").unwrap();

        let err = write_sounds(dir.path(), false).unwrap_err();
        assert!(
            err.to_string().contains("warning.wav"),
            "error should name the file it refused to overwrite, got: {err}"
        );

        assert_eq!(
            std::fs::read(dir.path().join("warning.wav")).unwrap(),
            b"precious",
            "the pre-existing file must be left untouched"
        );
        assert!(
            !dir.path().join("beep.wav").exists(),
            "a refused run must not create any file, including ones earlier in the write order"
        );
        assert!(
            !dir.path().join("siren.wav").exists(),
            "a refused run must not create any file, including ones later in the write order"
        );
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

    /// The generated files must be readable by the daemon's service account
    /// whatever the caller's umask, since a sound the daemon cannot read fails
    /// silently.
    ///
    /// `umask(2)` is process-global and the test harness is multi-threaded, so
    /// setting it in-process would race every other test. Instead this test
    /// re-runs *itself* in a child process under `umask 0077`, which is what
    /// makes the assertion meaningful: under the usual 0022 the modes would come
    /// out right by accident.
    #[test]
    fn gen_sounds_writes_readable_files_whatever_the_umask() {
        const MARKER: &str = "ALERTU_GEN_SOUNDS_UMASK_CHILD";

        if std::env::var_os(MARKER).is_none() {
            let exe = std::env::current_exe().unwrap();
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "umask 0077; exec {exe:?} --exact --nocapture \
                     tests::gen_sounds_writes_readable_files_whatever_the_umask"
                ))
                .env(MARKER, "1")
                .status()
                .expect("re-running the test binary under a restrictive umask");
            assert!(status.success(), "the run under umask 0077 failed");
            return;
        }

        // Child: a fresh subdirectory, so `create_dir_all` really creates it.
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("alertu");

        write_sounds(&dir, false).unwrap();

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode(&dir),
            DIR_MODE,
            "the destination directory must not inherit the umask"
        );
        for name in ["beep.wav", "warning.wav", "siren.wav"] {
            assert_eq!(
                mode(&dir.join(name)),
                FILE_MODE,
                "{name} must not inherit the umask"
            );
        }
    }

    /// `fs::write` leaves an existing file's mode alone, so `--force` onto a
    /// tightly-permissioned file must still end up world-readable.
    #[test]
    fn gen_sounds_force_relaxes_the_mode_of_an_overwritten_file() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["beep.wav", "warning.wav", "siren.wav"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"precious").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        write_sounds(dir.path(), true).unwrap();

        for name in ["beep.wav", "warning.wav", "siren.wav"] {
            let mode = std::fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, FILE_MODE, "{name} kept its old restrictive mode");
        }
    }

    #[test]
    fn gen_sounds_force_overwrites_all_three() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["beep.wav", "warning.wav", "siren.wav"] {
            std::fs::write(dir.path().join(name), b"precious").unwrap();
        }

        write_sounds(dir.path(), true).unwrap();

        for name in ["beep.wav", "warning.wav", "siren.wav"] {
            assert_ne!(
                std::fs::read(dir.path().join(name)).unwrap(),
                b"precious",
                "{name} should have been overwritten with --force"
            );
        }
    }
}
