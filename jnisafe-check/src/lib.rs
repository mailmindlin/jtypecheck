//! `jnisafe-check` — validate that a Rust/Java JNI layer agrees on symbol
//! names, FFI signatures, Java object types, and `@Ref`/`@Mut`/`@Owned`
//! pointer annotations.
//!
//! The two front-ends ([`java_loader`], [`rust_loader`]) lower into a shared IR
//! ([`ir`]); [`check`] compares them and produces a [`diagnostics::Report`].

pub mod cfg;
pub mod check;
pub mod cli;
pub mod code;
pub mod diagnostics;
pub mod flow;
pub mod ir;
pub mod java_loader;
pub mod jdk;
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
    let mut java_models = java_loader::load_models(&cfg.java)?;
    let rust = rust_loader::SynBackend.extract(&cfg.rust_crate)?;

    // Pull in any JDK stdlib classes a `bind_java_type!` binding references but
    // that weren't passed on `--java` (e.g. `java.nio.ByteBuffer`), resolved from
    // the JDK at `--java-home`/`$JAVA_HOME`.
    resolve_jdk_models(cfg, &rust.java_refs, &mut java_models)?;

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
    // Java-side handle-flow analysis over method bodies (opt-in via `--flow`).
    if cfg.flow {
        flow::analyze(&cfg.java, &mut report)?;
    }
    Ok(report)
}

/// Resolve the JDK stdlib classes referenced by `bind_java_type!` bindings that
/// were not supplied on `--java`, appending their models (and supertype closure)
/// to `java_models`. A no-op when no such classes are referenced or no JDK home
/// is available — the affected bindings then surface as W004.
fn resolve_jdk_models(
    cfg: &cli::Config,
    java_refs: &[ir::JavaRef],
    java_models: &mut Vec<ir::JavaClassModel>,
) -> Result<(), RunError> {
    use std::collections::HashSet;

    let user_names: HashSet<String> = java_models
        .iter()
        .map(|m| m.internal_name.clone())
        .collect();

    // Seeds: for each referenced class, the class itself when it wasn't provided
    // on `--java` (a direct JDK binding); or, when it *was* provided, its
    // supertypes (so a user class extending a JDK type gets its inherited members
    // resolved). `load_jdk_models` pulls in the rest of each chain.
    let mut seeds: HashSet<String> = HashSet::new();
    for r in java_refs {
        match java_models
            .iter()
            .find(|m| m.internal_name == r.class_internal)
        {
            Some(m) => {
                seeds.extend(m.super_class.clone());
                seeds.extend(m.interfaces.iter().cloned());
            }
            None => {
                seeds.insert(r.class_internal.clone());
            }
        }
    }
    seeds.retain(|s| !user_names.contains(s));
    if seeds.is_empty() {
        return Ok(());
    }

    let Some(home) = cfg.effective_java_home() else {
        return Ok(());
    };
    let jdk = java_loader::load_jdk_models(&home, &seeds, &user_names)?;
    java_models.extend(jdk);
    Ok(())
}
