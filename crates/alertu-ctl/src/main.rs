//! AlertU command-line control.
//!
//! A thin, scriptable wrapper over the daemon socket: everything the tray and
//! the settings window can do, plus a `--json` mode that emits the raw
//! protocol responses so shell scripts can consume them.

#![forbid(unsafe_code)]

mod cli;
mod commands;
mod error;
mod render;
mod sounds;

use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = cli::Cli::parse();
    match cli::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("alertu-ctl: {:#}", e.source);
            ExitCode::from(e.code)
        }
    }
}
