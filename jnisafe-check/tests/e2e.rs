//! End-to-end + loader tests driving the public library API against the
//! fixtures (`tests/fixtures/classes/*.class`, `tests/fixtures/*/wrong.rs`) and
//! the workspace `example/rust` crate.
//!
//! The `.class` files under `tests/fixtures/classes/` are generated, not
//! committed (see `.gitignore`). `pixi run test` regenerates them before the
//! suite runs; a bare `cargo test` needs them produced first, via:
//!   pixi run fixtures
//! If they are absent the tests fail with a message pointing here.

use std::path::PathBuf;

use jnisafe_check::cli::{Config, Format};
use jnisafe_check::ir::{IrType, PointerKind, Receiver};
use jnisafe_check::rust_loader::{RustExtractor, SynBackend};
use jnisafe_check::{java_loader, run};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(rel: &str) -> PathBuf {
    root().join(rel)
}

/// Resolve a generated `.class` fixture, failing with an actionable hint if the
/// classes have not been compiled yet (they are gitignored build artifacts).
fn class(rel: &str) -> PathBuf {
    let path = fixture(rel);
    assert!(
        path.exists(),
        "missing generated fixture: {}\n\
         The `.class` files under tests/fixtures/classes/ are not committed.\n\
         Run `pixi run fixtures` (or `pixi run test`) to compile them, then re-run.",
        path.display()
    );
    path
}

fn example_rust() -> PathBuf {
    root().join("../example/rust")
}

fn config(rust_crate: PathBuf, java: Vec<PathBuf>) -> Config {
    Config {
        rust_crate,
        java,
        format: Format::Human,
        quiet: true,
        verbose: false,
    }
}

#[test]
fn correct_passes() {
    // example/rust implements `HandWritten` as hand-written `Java_*` exports and
    // the remaining classes via the jni macros (`#[jni_mangle]`, `native_method!`,
    // `bind_java_type!`, plus two overloaded `native_method!` natives for
    // `Overloaded`); all must match cleanly.
    let cfg = config(
        example_rust(),
        vec![
            class("tests/fixtures/classes/example/HandWritten.class"),
            class("tests/fixtures/classes/example/Document.class"),
            class("tests/fixtures/classes/example/Mangle.class"),
            class("tests/fixtures/classes/example/NativeMethod.class"),
            class("tests/fixtures/classes/example/BindType.class"),
            class("tests/fixtures/classes/example/Overloaded.class"),
        ],
    );
    let report = run(&cfg).expect("run");
    assert!(!report.has_errors(), "expected no errors, got:\n{report}");
    assert_eq!(report.diagnostics.len(), 0);
}

#[test]
fn incorrect_reports_one_diagnostic_per_case() {
    let cfg = config(
        fixture("tests/fixtures/incorrect"),
        vec![class("tests/fixtures/classes/example/Incorrect.class")],
    );
    let report = run(&cfg).expect("run");

    for code in [
        "E001", "E023", "E024", "E025", "E020", "E021", "E010", "E002", "W001", "E026", "E004",
        "W002", "W003",
    ] {
        assert!(report.has_code(code), "missing {code}; report was:\n{report}");
    }
    assert!(report.has_errors());
    assert_eq!(report.error_count(), 10);
    assert_eq!(report.warning_count(), 3);
}

#[test]
fn incorrect_macros_report_expected_diagnostics() {
    // Macro-declared natives are validated like hand-written exports: one
    // diagnostic per deliberately-wrong method across the three macro forms.
    let cfg = config(
        fixture("tests/fixtures/incorrect_macros"),
        vec![class(
            "tests/fixtures/classes/example/IncorrectMacros.class",
        )],
    );
    let report = run(&cfg).expect("run");

    for code in ["E023", "E024", "E021", "W003"] {
        assert!(report.has_code(code), "missing {code}; report was:\n{report}");
    }
    assert!(report.has_errors());
    assert_eq!(report.error_count(), 3);
    assert_eq!(report.warning_count(), 1);
}

#[test]
fn incorrect_calls_report_expected_diagnostics() {
    // The Rust→Java direction: `bind_java_type!`'s methods/fields/constructors
    // clauses are verified against the Java class. Each deliberately-wrong entry
    // isolates one diagnostic, plus a binding to an unloaded class (W004).
    let cfg = config(
        fixture("tests/fixtures/incorrect_calls"),
        vec![class("tests/fixtures/classes/example/IncorrectCalls.class")],
    );
    let report = run(&cfg).expect("run");

    for code in ["E040", "E041", "E042", "E043", "E044", "W004"] {
        assert!(report.has_code(code), "missing {code}; report was:\n{report}");
    }
    assert!(report.has_errors());
    assert_eq!(report.error_count(), 5);
    assert_eq!(report.warning_count(), 1);
}

#[test]
fn field_handle_annotations_report_expected_diagnostics() {
    // The Rust→Java direction for *handle fields*: `bind_java_type!`'s
    // `fields { … }` declares `long` fields as `JOwned`/`JRef`/`JMut` handles, and
    // each is cross-checked against the Java field's `@Owned`/`@Ref`/`@Mut`
    // annotation. `cached` matches cleanly; `bare` is an unannotated handle field
    // (W005); `wrong` is annotated with the wrong pointee type (E045).
    let cfg = config(
        fixture("tests/fixtures/field_handles"),
        vec![class("tests/fixtures/classes/example/FieldHandles.class")],
    );
    let report = run(&cfg).expect("run");

    for code in ["W005", "E045"] {
        assert!(report.has_code(code), "missing {code}; report was:\n{report}");
    }
    // `cached` is a clean match: no field existence/type errors fire.
    assert!(!report.has_code("E042"), "{report}");
    assert!(!report.has_code("E043"), "{report}");
    assert_eq!(report.error_count(), 1);
    assert_eq!(report.warning_count(), 1);
}

#[test]
fn overloaded_macro_methods_match_when_supported() {
    // Two overloaded `native_method!` impls match their long-form overloaded
    // Java declarations: the Rust front-end's `resolve_overloads` pass re-mangles
    // same-named macro natives to the same `..._combine__<args>` symbols
    // `java_loader` produces, so they pair up cleanly with no collision.
    let cfg = config(
        fixture("tests/fixtures/overloaded"),
        vec![class("tests/fixtures/classes/example/Overloaded.class")],
    );
    let report = run(&cfg).expect("run");
    assert!(
        !report.has_errors(),
        "overloaded macro natives should match their Java declarations:\n{report}");
}

#[test]
fn json_output_carries_codes() {
    let cfg = config(
        fixture("tests/fixtures/incorrect"),
        vec![class("tests/fixtures/classes/example/Incorrect.class")],
    );
    let report = run(&cfg).expect("run");
    let json = report.render_json();
    // Each line must be valid JSON; the kind-mismatch code must appear.
    let mut saw_e023 = false;
    for line in json.lines() {
        let v: serde_json::Value = serde_json::from_str(line).expect("valid json line");
        if v.get("code").and_then(|c| c.as_str()) == Some("E023") {
            saw_e023 = true;
        }
    }
    assert!(saw_e023, "E023 not present in JSON output:\n{json}");
}

#[test]
fn java_loader_reads_pointer_annotations() {
    let sigs =
        java_loader::load(&[class("tests/fixtures/classes/example/HandWritten.class")]).unwrap();

    let find = |method: &str| {
        sigs.iter()
            .find(|s| s.key.java_method == method)
            .expect(method)
    };

    // tryGet(@Ref("Box<String>") long) — default nullable=true.
    let try_get = find("tryGet");
    match &try_get.params[0] {
        IrType::Pointer(p) => {
            assert_eq!(p.kind, PointerKind::Ref);
            assert_eq!(p.rust_type, "Box<String>");
            assert!(p.nullable, "default nullable should be true");
        }
        other => panic!("expected pointer, got {other:?}"),
    }

    // get(@Ref(..., nullable=false) long) — explicit nullable=false.
    match &find("get").params[0] {
        IrType::Pointer(p) => assert!(!p.nullable, "explicit nullable=false"),
        other => panic!("expected pointer, got {other:?}"),
    }

    // create returns @Owned("Box<String>") long.
    match &find("create").ret {
        IrType::Pointer(p) => {
            assert_eq!(p.kind, PointerKind::Owned);
            assert_eq!(p.rust_type, "Box<String>");
        }
        other => panic!("expected owned pointer return, got {other:?}"),
    }
    // ...and takes a String param.
    assert_eq!(
        find("create").params[0],
        IrType::JavaObject {
            class: "java/lang/String".to_owned()
        }
    );

    assert!(find("create").is_static);
    assert_eq!(find("create").key.symbol, "Java_example_HandWritten_create");
}

#[test]
fn rust_loader_skips_env_and_receiver() {
    let sigs = SynBackend.extract(&example_rust()).unwrap().natives;
    let find = |sym: &str| sigs.iter().find(|s| s.key.symbol == sym).expect(sym);

    // create(EnvUnowned, JClass, JString) -> JOwned<Box<String>>
    let create = find("Java_example_HandWritten_create");
    assert_eq!(create.receiver, Receiver::Class);
    assert_eq!(create.params.len(), 1, "env + class skipped");
    assert_eq!(
        create.params[0],
        IrType::JavaObject {
            class: "java/lang/String".to_owned()
        }
    );
    match &create.ret {
        IrType::Pointer(p) => {
            assert_eq!(p.kind, PointerKind::Owned);
            assert_eq!(p.rust_type, "Box<String>");
        }
        other => panic!("expected owned return, got {other:?}"),
    }

    // tryGet takes Option<JRef<..>> => nullable.
    match &find("Java_example_HandWritten_tryGet").params[0] {
        IrType::Pointer(p) => assert!(p.nullable, "Option<JRef> => nullable"),
        other => panic!("expected pointer, got {other:?}"),
    }
    // get takes a bare JRef => non-nullable.
    match &find("Java_example_HandWritten_get").params[0] {
        IrType::Pointer(p) => assert!(!p.nullable, "bare JRef => non-nullable"),
        other => panic!("expected pointer, got {other:?}"),
    }

    // Param counts after skipping env+receiver: create=1, tryGet=1, get=1, set=2, drop=1.
    assert_eq!(find("Java_example_HandWritten_set").params.len(), 2);
    assert_eq!(find("Java_example_HandWritten_drop").params.len(), 1);
}
