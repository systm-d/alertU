//! Writing the default sound files into a directory.
//!
//! The one subcommand that never opens the socket, so every way it can fail is
//! a local input error.

use super::print_outcome;
use crate::error::CliError;
use crate::render::Outcome;
use crate::sounds;
use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// The arm/disarm chirp and the alarm siren, embedded so `gen-sounds` needs no
/// data files at runtime. Both are recordings rather than synthesis: they simply
/// sound better than a sine sweep. Only the countdown tick is still generated,
/// because it has to be a few tens of milliseconds — `play_once` spawns a player
/// once a second, so anything longer would pile up.
///
/// They live under `assets/` rather than the repository's `resources/` because
/// `cargo package` only includes files inside the crate; reaching outside it
/// would make this crate unpublishable.
const BEEP_WAV: &[u8] = include_bytes!("../../assets/lock.wav");
const SIREN_WAV: &[u8] = include_bytes!("../../assets/siren.wav");

/// Modes forced onto the destination directory and the files written into it.
///
/// Explicit, because the documented install step is
/// `sudo alertu-ctl gen-sounds --dir /usr/share/sounds/alertu`: with a umask of
/// `0027` the defaults would land as root-owned 0750/0640, which the
/// unprivileged `alertu` service account cannot read. That failure is silent —
/// `sound.rs::play_once` only checks `path.exists()` and sends the player's
/// stderr to `/dev/null` — so all audio would simply disappear.
pub(crate) const DIR_MODE: u32 = 0o755;
pub(crate) const FILE_MODE: u32 = 0o644;

/// Write the default sounds into `dir` and acknowledge.
pub fn run(json: bool, dir: &Path, force: bool) -> Result<(), CliError> {
    write_sounds(dir, force).map_err(CliError::usage)?;
    print_outcome(&Outcome::Ack, json)?;
    Ok(())
}

/// Write the three default sounds into `dir`.
///
/// Refuses to overwrite unless `force`, because the natural destination is a
/// system directory a user may have customised.
pub(crate) fn write_sounds(dir: &Path, force: bool) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The generated files must be readable by the daemon's service account
    /// whatever the caller's umask, since a sound the daemon cannot read fails
    /// silently.
    ///
    /// `umask(2)` is process-global and the test harness is multi-threaded, so
    /// setting it in-process would race every other test. Instead this test
    /// re-runs *itself* in a child process under `umask 0077`, which is what
    /// makes the assertion meaningful: under the usual 0022 the modes would come
    /// out right by accident.
    ///
    /// The test path below is spelled out for the child, so it must track this
    /// module's location. If it ever stops matching, the child runs no test at
    /// all, exits 0, and this test passes while proving nothing.
    #[test]
    fn gen_sounds_writes_readable_files_whatever_the_umask() {
        const MARKER: &str = "ALERTU_GEN_SOUNDS_UMASK_CHILD";
        const TEST_PATH: &str =
            "commands::gen_sounds::tests::gen_sounds_writes_readable_files_whatever_the_umask";

        if std::env::var_os(MARKER).is_none() {
            let exe = std::env::current_exe().unwrap();
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "umask 0077; exec {exe:?} --exact --nocapture {TEST_PATH}"
                ))
                .env(MARKER, "1")
                .output()
                .expect("re-running the test binary under a restrictive umask");
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "the run under umask 0077 failed:\n{stdout}"
            );
            // A stale `TEST_PATH` would filter every test out, and a run of
            // nothing exits 0 — which would make this test vacuous.
            assert!(
                stdout.contains("1 passed"),
                "the child ran no test; is TEST_PATH stale?\n{stdout}"
            );
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
