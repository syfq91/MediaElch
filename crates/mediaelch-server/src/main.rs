//! Headless MediaElch server binary.

mod cli;
mod config;

use std::process::ExitCode;

use clap::Parser;
use cli::Cli;
use config::Config;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let config = Config::load(&cli)?;

    if cli.check {
        println!("{}", config.summary());
        for warning in config.warnings() {
            println!("warning: {warning}");
        }
        return Ok(());
    }

    println!("configuration loaded");
    println!("{}", config.summary());
    Ok(())
}
