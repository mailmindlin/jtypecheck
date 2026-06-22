//! `jnisafe-check` — validate that a Rust/Java JNI layer agrees on symbol
//! names, FFI signatures, Java object types, and `@Ref`/`@Mut`/`@Owned`
//! pointer annotations.
//!
//! The two front-ends ([`java_loader`], [`rust_loader`]) lower into a shared IR
//! ([`ir`]); [`check`] compares them and produces a [`diagnostics::Report`].

pub mod check;
pub mod cli;
pub mod diagnostics;
pub mod ir;
pub mod java_loader;
pub mod mangle;
pub mod rust_loader;
pub mod typemap;

use diagnostics::Report;
use rust_loader::RustExtractor;

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Java(#[from] java_loader::JavaLoadError),
    #[error(transparent)]
    Rust(#[from] rust_loader::RustLoadError),
}

/// Load both sides and run the checker.
pub fn run(cfg: &cli::Config) -> Result<Report, RunError> {
    let java_sigs = java_loader::load(&cfg.java)?;
    let rust_sigs = rust_loader::SynBackend.extract(&cfg.rust_crate)?;

    if cfg.verbose {
        eprintln!("== Java signatures ==");
        for s in &java_sigs {
            eprintln!("  {s:?}");
        }
        eprintln!("== Rust signatures ==");
        for s in &rust_sigs {
            eprintln!("  {s:?}");
        }
    }

    Ok(check::check(&java_sigs, &rust_sigs))
}
