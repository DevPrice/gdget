use std::process::ExitCode;

use clap::Parser;
use gdget::cli::Cli;
use gdget::report::Reporter;
use gdget::{Outcome, run};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let reporter = Reporter::from_env();
    match run(cli, reporter) {
        Ok(Outcome::Success) => ExitCode::SUCCESS,
        Ok(Outcome::Failure) => ExitCode::FAILURE,
        Err(err) => {
            reporter.error(format!("{err:#}"));
            ExitCode::FAILURE
        }
    }
}
