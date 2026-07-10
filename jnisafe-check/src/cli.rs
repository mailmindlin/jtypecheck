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
// New flags are added as fields over time; `#[non_exhaustive]` keeps that from
// breaking library callers. Build one with [`Config::parse`] (CLI) or
// [`Config::new`] (programmatic), then set the public fields you need.
#[non_exhaustive]
pub struct Config {
    /// Path to the Rust crate root (its `src/**/*.rs` is scanned), or a single
    /// `.rs` file.
    #[arg(long)]
    pub rust_crate: PathBuf,

    /// Java input(s): a `.class` file, a directory of classes, or a `.jar`.
    /// Repeatable.
    #[arg(long = "java", required = true, num_args = 1..)]
    pub java: Vec<PathBuf>,

    /// A JDK home (e.g. `$JAVA_HOME`), used to resolve JDK stdlib classes that a
    /// `bind_java_type!` binding references but that are not passed on `--java`
    /// (e.g. `java.nio.ByteBuffer`). Needs a full JDK — its `jmods/` (Java 9+) or
    /// `rt.jar` (Java 8) — not a JRE. Defaults to the `JAVA_HOME` environment
    /// variable; if neither is set, such bindings can't be verified (W004).
    #[arg(long)]
    pub java_home: Option<PathBuf>,

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

impl Config {
    /// Build a `Config` programmatically (the CLI uses the derived
    /// [`Config::parse`]). Everything but the two required inputs takes its CLI
    /// default; set the remaining public fields afterwards as needed. `java_home`
    /// defaults to `None`, so [`Config::effective_java_home`] still falls back to
    /// `$JAVA_HOME`.
    pub fn new(rust_crate: PathBuf, java: Vec<PathBuf>) -> Self {
        Config {
            rust_crate,
            java,
            java_home: None,
            flow: false,
            format: Format::Human,
            quiet: false,
            verbose: false,
        }
    }

    /// The effective JDK home for resolving stdlib classes: the `--java-home`
    /// flag if given, else the `JAVA_HOME` environment variable, else `None`.
    pub fn effective_java_home(&self) -> Option<PathBuf> {
        self.java_home
            .clone()
            .or_else(|| std::env::var_os("JAVA_HOME").map(PathBuf::from))
    }
}
