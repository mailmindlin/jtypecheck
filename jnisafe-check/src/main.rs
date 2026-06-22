use std::process::ExitCode;

use clap::Parser;
use jnisafe_check::cli::{Config, Format};
use jnisafe_check::run;

fn main() -> ExitCode {
    let cfg = Config::parse();

    let report = match run(&cfg) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(3);
        }
    };

    match cfg.format {
        Format::Human => {
            if report.diagnostics.is_empty() {
                if !cfg.quiet {
                    println!("ok: all native methods matched");
                }
            } else {
                print!("{}", report.render_human());
            }
        }
        Format::Json => print!("{}", report.render_json()),
    }

    if report.has_errors() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
