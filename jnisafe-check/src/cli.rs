//! Command-line surface.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// rustc-style human-readable diagnostics (default).
    Human,
    /// One JSON object per line, plus a trailing summary object.
    Json,
}

#[derive(Debug, Parser)]
#[command(
    name = "jnisafe-check",
    about = "Validate that a Rust/Java JNI layer agrees on names, signatures, and pointer types."
)]
pub struct Config {
    /// Path to the Rust crate root (its `src/**/*.rs` is scanned), or a single
    /// `.rs` file.
    #[arg(long)]
    pub rust_crate: PathBuf,

    /// Java input(s): a `.class` file, a directory of classes, or a `.jar`.
    /// Repeatable.
    #[arg(long = "java", required = true, num_args = 1..)]
    pub java: Vec<PathBuf>,

    /// Also run the Java-side handle-flow analysis (leaks, use-after-move,
    /// forging, wrong-type, exposure, …) over the method bodies. Off by default:
    /// it is intraprocedural and can flag legitimate-but-unannotated handle
    /// patterns, so it is opt-in.
    #[arg(long)]
    pub flow: bool,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Human)]
    pub format: Format,

    /// Suppress the success summary on a clean run.
    #[arg(short, long)]
    pub quiet: bool,

    /// Dump the extracted IR for both sides before checking.
    #[arg(short, long)]
    pub verbose: bool,
}
