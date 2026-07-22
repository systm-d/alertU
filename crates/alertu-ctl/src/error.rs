//! The exit-code contract.
//!
//! Three outcomes a shell script can act on: success, the daemon said no (or
//! there was nothing listening), and you asked for something impossible.

/// Something went wrong on the daemon side: it refused the request, or there
/// was nothing listening on the socket at all.
pub const EXIT_DAEMON: u8 = 1;
/// Bad input from the caller, caught locally before any socket is touched.
/// Same code clap uses for a malformed command line.
pub const EXIT_USAGE: u8 = 2;

/// A failure together with the exit code it maps to, so the shell can tell
/// "the daemon said no" (1) from "you asked for something impossible" (2).
#[derive(Debug)]
pub struct CliError {
    pub code: u8,
    pub source: anyhow::Error,
}

impl CliError {
    /// Bad local input: exits 2, like clap's own usage errors.
    pub fn usage(source: anyhow::Error) -> CliError {
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
