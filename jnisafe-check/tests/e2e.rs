//! End-to-end + loader tests driving the public library API against the
//! committed fixtures (`tests/fixtures/classes/*.class`, `tests/fixtures/incorrect/wrong.rs`)
//! and the workspace `example/rust` crate.
//!
//! Regenerate the `.class` fixtures with:
//!   javac -d /tmp/ann ../jnisafe-annotations/io/github/mailmindlin/jnisafe/*.java
//!   javac -cp /tmp/ann -d tests/fixtures/classes ../example/java/example/*.java

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
    let cfg = config(
        example_rust(),
        vec![fixture("tests/fixtures/classes/example/Correct.class")],
    );
    let report = run(&cfg).expect("run");
    assert!(
        !report.has_errors(),
        "expected no errors, got:\n{}",
        report.render_human()
    );
    assert_eq!(report.diagnostics.len(), 0);
}

#[test]
fn incorrect_reports_one_diagnostic_per_case() {
    let cfg = config(
        fixture("tests/fixtures/incorrect"),
        vec![fixture("tests/fixtures/classes/example/Incorrect.class")],
    );
    let report = run(&cfg).expect("run");

    for code in [
        "E001", "E023", "E024", "E025", "E020", "E021", "E010", "E002", "W001", "E026", "E004",
        "W002", "W003",
    ] {
        assert!(
            report.has_code(code),
            "missing {code}; report was:\n{}",
            report.render_human()
        );
    }
    assert!(report.has_errors());
    assert_eq!(report.error_count(), 10);
    assert_eq!(report.warning_count(), 3);
}

#[test]
fn json_output_carries_codes() {
    let cfg = config(
        fixture("tests/fixtures/incorrect"),
        vec![fixture("tests/fixtures/classes/example/Incorrect.class")],
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
        java_loader::load(&[fixture("tests/fixtures/classes/example/Correct.class")]).unwrap();

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
    assert_eq!(find("create").key.symbol, "Java_example_Correct_create");
}

#[test]
fn rust_loader_skips_env_and_receiver() {
    let sigs = SynBackend.extract(&example_rust()).unwrap();
    let find = |sym: &str| sigs.iter().find(|s| s.key.symbol == sym).expect(sym);

    // create(EnvUnowned, JClass, JString) -> JOwned<Box<String>>
    let create = find("Java_example_Correct_create");
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
    match &find("Java_example_Correct_tryGet").params[0] {
        IrType::Pointer(p) => assert!(p.nullable, "Option<JRef> => nullable"),
        other => panic!("expected pointer, got {other:?}"),
    }
    // get takes a bare JRef => non-nullable.
    match &find("Java_example_Correct_get").params[0] {
        IrType::Pointer(p) => assert!(!p.nullable, "bare JRef => non-nullable"),
        other => panic!("expected pointer, got {other:?}"),
    }

    // Param counts after skipping env+receiver: create=1, tryGet=1, get=1, set=2, drop=1.
    assert_eq!(find("Java_example_Correct_set").params.len(), 2);
    assert_eq!(find("Java_example_Correct_drop").params.len(), 1);
}
