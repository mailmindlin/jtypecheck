//! End-to-end demo for `pixi run demo`.
//!
//! Drives the public library API (the same surface as `tests/e2e.rs`) to show
//! the whole point of the checker in one run:
//!   * a **positive** phase — the correct example classes match `example/rust`
//!     cleanly (zero diagnostics), and
//!   * four **negative** phases — the intentionally-wrong `Incorrect` /
//!     `IncorrectMacros` / `IncorrectCalls` / `FieldHandles` classes are rejected
//!     with the *exact* diagnostics we expect.
//!
//! Paths are resolved relative to the current working directory; pixi runs tasks
//! from the workspace root, and `pixi run demo` depends on `fixtures`, which
//! compiles the example + fixture Java into the (gitignored)
//! `jnisafe-check/tests/fixtures/classes/example/*.class` — the same classes the
//! test suite uses.
//!
//! Run with: `pixi run demo` (or `cargo run -p jnisafe-check --example demo`
//! after `pixi run fixtures`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use jnisafe_check::cli::{Config, Format};
use jnisafe_check::run;

fn config(rust_crate: &str, java: &[&str]) -> Config {
    Config {
        rust_crate: PathBuf::from(rust_crate),
        java: java.iter().map(PathBuf::from).collect(),
        format: Format::Human,
        quiet: true,
        verbose: false,
    }
}

/// A negative phase's expectations — kept in sync with `tests/e2e.rs`.
struct Expect {
    codes: &'static [&'static str],
    errors: usize,
    warnings: usize,
}

fn main() -> ExitCode {
    // Fail early with a clear hint if the classes were not compiled.
    let probe = Path::new("jnisafe-check/tests/fixtures/classes/example/HandWritten.class");
    if !probe.exists() {
        eprintln!(
            "demo: missing {}\n      run `pixi run demo` (it compiles the Java first), \
             or `pixi run fixtures` then re-run.",
            probe.display()
        );
        return ExitCode::from(2);
    }

    let mut ok = true;

    // --- Phase 1: positive ---------------------------------------------------
    println!("== positive: correct Java/Rust layers should match ==");
    let cfg = config(
        "example/rust",
        &[
            "jnisafe-check/tests/fixtures/classes/example/HandWritten.class",
            "jnisafe-check/tests/fixtures/classes/example/Document.class",
            "jnisafe-check/tests/fixtures/classes/example/Mangle.class",
            "jnisafe-check/tests/fixtures/classes/example/NativeMethod.class",
            "jnisafe-check/tests/fixtures/classes/example/BindType.class",
            "jnisafe-check/tests/fixtures/classes/example/Overloaded.class",
        ],
    );
    match run(&cfg) {
        Ok(report) => {
            if report.diagnostics.is_empty() {
                println!("ok: all native methods matched (0 diagnostics)\n");
            } else {
                ok = false;
                eprintln!(
                    "FAIL: expected a clean run, got:\n{}",
                    report.render_human()
                );
            }
        }
        Err(e) => {
            ok = false;
            eprintln!("FAIL: positive phase errored: {e}");
        }
    }

    // --- Phase 2: negative (hand-written exports) ----------------------------
    ok &= negative_phase(
        "negative: hand-written exports — Incorrect should be rejected",
        "jnisafe-check/tests/fixtures/incorrect",
        "jnisafe-check/tests/fixtures/classes/example/Incorrect.class",
        &Expect {
            codes: &[
                "E001", "E023", "E024", "E025", "E020", "E021", "E010", "E002", "W001", "E026",
                "E004", "W002", "W003",
            ],
            errors: 10,
            warnings: 3,
        },
    );

    // --- Phase 3: negative (macro-declared natives) --------------------------
    ok &= negative_phase(
        "negative: macro-declared natives — IncorrectMacros should be rejected",
        "jnisafe-check/tests/fixtures/incorrect_macros",
        "jnisafe-check/tests/fixtures/classes/example/IncorrectMacros.class",
        &Expect {
            codes: &["E023", "E024", "E021", "W003"],
            errors: 3,
            warnings: 1,
        },
    );

    // --- Phase 4: negative (Rust→Java call bindings) -------------------------
    ok &= negative_phase(
        "negative: Rust→Java bindings — IncorrectCalls should be rejected",
        "jnisafe-check/tests/fixtures/incorrect_calls",
        "jnisafe-check/tests/fixtures/classes/example/IncorrectCalls.class",
        &Expect {
            codes: &["E040", "E041", "E042", "E043", "E044", "W004"],
            errors: 5,
            warnings: 1,
        },
    );

    // --- Phase 5: negative (Rust→Java handle fields) ------------------------
    ok &= negative_phase(
        "negative: handle fields — FieldHandles annotations should be checked",
        "jnisafe-check/tests/fixtures/field_handles",
        "jnisafe-check/tests/fixtures/classes/example/FieldHandles.class",
        &Expect {
            codes: &["W005", "E045"],
            errors: 1,
            warnings: 1,
        },
    );

    if ok {
        println!("demo: all phases passed — correct layers accepted, broken layers rejected.");
        ExitCode::SUCCESS
    } else {
        eprintln!("demo: one or more phases did not match expectations.");
        ExitCode::from(1)
    }
}

/// Run a negative phase: print the caught diagnostics, then assert the exact
/// codes and counts. Returns `true` when every expectation is met.
fn negative_phase(title: &str, rust_crate: &str, java: &str, expect: &Expect) -> bool {
    println!("== {title} ==");
    let report = match run(&config(rust_crate, &[java])) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("FAIL: phase errored: {e}\n");
            return false;
        }
    };

    // Show what the checker caught.
    print!("{}", report.render_human());

    let mut ok = true;
    for &code in expect.codes {
        if !report.has_code(code) {
            ok = false;
            eprintln!("FAIL: missing expected diagnostic {code}");
        }
    }
    check_count("error", report.error_count(), expect.errors, &mut ok);
    check_count("warning", report.warning_count(), expect.warnings, &mut ok);

    if ok {
        println!("ok: rejected with the expected diagnostics\n");
    } else {
        eprintln!();
    }
    ok
}

fn check_count(kind: &str, got: usize, want: usize, ok: &mut bool) {
    if got != want {
        *ok = false;
        eprintln!("FAIL: expected {want} {kind}(s), got {got}");
    }
}
