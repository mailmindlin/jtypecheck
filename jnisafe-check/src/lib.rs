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
    let java_models = java_loader::load_models(&cfg.java)?;
    let rust = rust_loader::SynBackend.extract(&cfg.rust_crate)?;

    if cfg.verbose {
        eprintln!("== Java signatures ==");
        for s in &java_sigs {
            eprintln!("  {s:?}");
        }
        eprintln!("== Rust signatures ==");
        for s in &rust.natives {
            eprintln!("  {s:?}");
        }
        eprintln!("== Rust→Java references ==");
        for r in &rust.java_refs {
            eprintln!("  {r:?}");
        }
    }

    // Java→Rust: pair native methods by mangled symbol and compare signatures.
    let mut report = check::check(&java_sigs, &rust.natives);
    // Rust→Java: verify the methods/fields/constructors `bind_java_type!` calls
    // exist in the loaded classes.
    check::check_java_refs(&rust.java_refs, &java_models, &mut report);
    Ok(report)
}
