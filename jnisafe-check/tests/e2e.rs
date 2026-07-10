//! End-to-end + loader tests driving the public library API against the
//! fixtures (`tests/fixtures/classes/*.class`, `tests/fixtures/*/wrong.rs`) and
//! the workspace `example/rust` crate.
//!
//! The `.class` files under `tests/fixtures/classes/` are generated, not
//! committed (see `.gitignore`). `pixi run test` regenerates them before the
//! suite runs; a bare `cargo test` needs them produced first, via:
//!   pixi run fixtures
//! If they are absent the tests fail with a message pointing here.

use std::collections::HashSet;
use std::path::PathBuf;

use jnisafe_check::cli::Config;
use jnisafe_check::diagnostics::{Diagnostic, Report};
use jnisafe_check::ir::{IrType, PointerKind, Receiver};
use jnisafe_check::rust_loader::{RustExtractor, SynBackend};
use jnisafe_check::{java_loader, run};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(rel: &str) -> PathBuf {
    let mut root = root();
    root.push(rel);
    root
}

/// Resolve a generated `.class` fixture, failing with an actionable hint if the
/// classes have not been compiled yet (they are gitignored build artifacts).
#[track_caller]
fn class(rel: &str) -> PathBuf {
    let mut path = root();
    path.push("tests/fixtures/classes");
    path.push(rel);

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
    let mut cfg = Config::new(rust_crate, java);
    cfg.quiet = true;
    // `java_home` stays `None`; `effective_java_home()` falls back to `$JAVA_HOME`
    // so JDK stdlib bindings still resolve where the JDK tests need them.
    cfg
}

/// A usable JDK home for the JDK-resolution tests: `$JAVA_HOME` if set and it
/// contains a `jmods/` directory (a full JDK, as pixi provides). `None` under a
/// bare `cargo test` with no JDK, so those tests skip rather than fail.
fn jdk_home() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("JAVA_HOME")?);
    home.join("jmods").is_dir().then_some(home)
}

/// The member a diagnostic pins to: the Java method/field it fires on when it
/// carries a Java location (native-method and flow findings), otherwise the Rust
/// symbol (export-shape lints and Rust→Java `bind_java_type!` bindings).
fn locus(d: &Diagnostic) -> &str {
    if let Some(j) = &d.java {
        &j.method
    } else if let Some(r) = &d.rust {
        &r.symbol
    } else {
        panic!("diagnostic {} carries no location to pin", d.code);
    }
}

/// Assert a report's diagnostics are *exactly* `expected` — every `(code, locus)`
/// pair present, with no extras and nothing missing (order-independent). Pins
/// each finding to the member it fires on, subsuming a code-presence + count
/// check. Prefer the [`assert_findings!`] sugar.
#[track_caller]
fn check_findings(report: &Report, expected: &[(&str, &str)]) {
    let mut got: Vec<(&str, &str)> = report
        .diagnostics
        .iter()
        .map(|d| (d.code.as_str(), locus(d)))
        .collect();
    got.sort_unstable();
    let mut want: Vec<(&str, &str)> = expected.to_vec();
    want.sort_unstable();
    assert_eq!(got, want, "diagnostics mismatch:\n{report}");
}

/// `assert_findings!(report, "E001" @ "createWrongType", …)` — each `code @ member`
/// pins one diagnostic to the Java method/field it fires on, or to its Rust
/// symbol when it has no Java location. Asserts the listed findings are *exactly*
/// what the report contains (see [`check_findings`]).
macro_rules! assert_findings {
    ($report:expr $(, $code:literal @ $member:literal)* $(,)?) => {
        check_findings(&$report, &[$(($code, $member)),*])
    };
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
            class("example/HandWritten.class"),
            class("example/Document.class"),
            class("example/Mangle.class"),
            class("example/NativeMethod.class"),
            class("example/BindType.class"),
            class("example/Overloaded.class"),
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
        vec![class("example/Incorrect.class")],
    );
    let report = run(&cfg).expect("run");

    // One finding per native method, pinned to the method it fires on; the four
    // export-shape lints (W001/W002/W003/E004) have no Java method, so they pin
    // to the Rust symbol instead.
    assert_findings!(
        report,
        "E001" @ "createWrongType",
        "E023" @ "kindMismatch",
        "E024" @ "typeMismatch",
        "E025" @ "nullMismatch",
        "E020" @ "objMismatch",
        "E021" @ "primMismatch",
        "E010" @ "arityMismatch",
        "E002" @ "recvMismatch",
        "E026" @ "badSlot",
        "W003" @ "Java_example_Incorrect_borrowReturn",
        "W001" @ "Java_example_Incorrect_orphan",
        "W002" @ "Java_example_Incorrect_notExported",
        "E004" @ "Java_example_Incorrect_tooFewParams",
    );
}

#[test]
fn incorrect_macros_report_expected_diagnostics() {
    // Macro-declared natives are validated like hand-written exports: one
    // diagnostic per deliberately-wrong method across the three macro forms.
    let cfg = config(
        fixture("tests/fixtures/incorrect_macros"),
        vec![class("example/IncorrectMacros.class")],
    );
    let report = run(&cfg).expect("run");

    assert_findings!(
        report,
        "E023" @ "kindMismatch",
        "E024" @ "typeMismatch",
        "E021" @ "primMismatch",
        "W003" @ "Java_example_IncorrectMacros_borrowReturn",
    );
}

#[test]
fn incorrect_calls_report_expected_diagnostics() {
    // The Rust→Java direction: `bind_java_type!`'s methods/fields/constructors
    // clauses are verified against the Java class. Each deliberately-wrong entry
    // isolates one diagnostic, plus a binding to an unloaded class (W004).
    let cfg = config(
        fixture("tests/fixtures/incorrect_calls"),
        vec![class("example/IncorrectCalls.class")],
    );
    let report = run(&cfg).expect("run");

    // Reverse-direction findings carry no Java location, so each pins to the
    // `class.member` Rust symbol the binding names.
    assert_findings!(
        report,
        "E040" @ "example.IncorrectCalls.ghostMethod",
        "E041" @ "example.IncorrectCalls.realMethod",
        "E042" @ "example.IncorrectCalls.missingField",
        "E043" @ "example.IncorrectCalls.instanceValue",
        "E044" @ "example.IncorrectCalls.<init>",
        "W004" @ "example.NotLoadedClass.whatever",
    );
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
        vec![class("example/FieldHandles.class")],
    );
    let report = run(&cfg).expect("run");

    // Exactly these two findings — which also proves `cached` is a clean match
    // (no E042/E043 field existence/type errors, no extra diagnostics).
    assert_findings!(
        report,
        "W005" @ "example.FieldHandles.bare",
        "E045" @ "example.FieldHandles.wrong",
    );
}

#[test]
fn overloaded_macro_methods_match_when_supported() {
    // Two overloaded `native_method!` impls match their long-form overloaded
    // Java declarations: the Rust front-end's `resolve_overloads` pass re-mangles
    // same-named macro natives to the same `..._combine__<args>` symbols
    // `java_loader` produces, so they pair up cleanly with no collision.
    let cfg = config(
        fixture("tests/fixtures/overloaded"),
        vec![class("example/Overloaded.class")],
    );
    let report = run(&cfg).expect("run");
    assert!(
        !report.has_errors(),
        "overloaded macro natives should match their Java declarations:\n{report}"
    );
}

#[test]
fn array_wrappers_match_their_java_declarations() {
    // Array-typed natives bound through the generic jni wrappers
    // (`JPrimitiveArray<T>` / `JObjectArray<E>`, incl. the default `Object`
    // element and a nested `byte[][]`) must map to the same descriptors as their
    // Java `int[]` / `String[]` / `Object[]` / `byte[][]` declarations, so every
    // method pairs cleanly with no diagnostics.
    let cfg = config(
        fixture("tests/fixtures/arrays"),
        vec![class("example/Arrays.class")],
    );
    let report = run(&cfg).expect("run");
    assert!(
        !report.has_errors(),
        "array-typed natives should match their Java declarations:\n{report}"
    );
    assert_eq!(
        report.diagnostics.len(),
        0,
        "unexpected findings:\n{report}"
    );
}

#[test]
fn object_array_of_bound_type_matches() {
    // A native returns/takes `Pose[]`, whose Rust element type `JPose` is a user
    // wrapper bound via `bind_java_type! { JPose => "example.Pose" }`. The checker
    // must resolve `JObjectArray<'local, JPose<'local>>` to `[Lexample/Pose;` and
    // match cleanly — element types aren't limited to built-ins like `JString`.
    let cfg = config(
        fixture("tests/fixtures/object_arrays"),
        vec![
            class("example/ObjectArrays.class"),
            class("example/Pose.class"),
        ],
    );
    let report = run(&cfg).expect("run");
    assert!(
        !report.has_errors(),
        "object-array natives with a bound element type should match:\n{report}"
    );
    assert_eq!(
        report.diagnostics.len(),
        0,
        "unexpected findings:\n{report}"
    );
}

#[test]
fn json_output_carries_codes() {
    let cfg = config(
        fixture("tests/fixtures/incorrect"),
        vec![class("example/Incorrect.class")],
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
    let sigs = java_loader::load(&[class("example/HandWritten.class")]).unwrap();

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

// ---- Java-side handle-flow analysis (flow.rs) -----------------------------
//
// These drive `flow::analyze` directly on a single fixture class, isolated from
// the boundary check, so the reported diagnostics are exactly the flow findings
// (no orphan-export / signature noise). They are the executable spec for the
// analysis and FAIL until each phase lands; treat them as the progress tracker.

use jnisafe_check::flow;

fn flow_report(class_rel: &str) -> Report {
    let mut report = Report::default();
    flow::analyze(&[class(class_rel)], &mut report).expect("flow analyze");
    report
}

#[test]
fn flow_basic_cases() {
    // test1..test5: one diagnostic each, pinned to its method.
    let r = flow_report("example/Flow.class");
    assert_findings!(
        r,
        "W010" @ "test1",
        "W011" @ "test2",
        "E061" @ "test3",
        "E062" @ "test4",
        "E063" @ "test5",
    );
}

#[test]
fn flow_forging() {
    // A fabricated constant, a literal-0-into-non-nullable, and an unannotated
    // `long` parameter used as a handle — all E060. (`nullIntoNullable`, a 0 into
    // a nullable @Owned slot, is fine.)
    let r = flow_report("example/Forge.class");
    assert_findings!(
        r,
        "E060" @ "fabricate",
        "E060" @ "nullIntoNonNullable",
        "E060" @ "externalFabricate",
    );
}

#[test]
fn flow_field_take_and_overwrite() {
    let r = flow_report("example/FieldTake.class");
    // E064: consumed (or escaped via return) while still installed in the field.
    // W012: per store-site — overwriteLive's two stores raise two.
    // E060: in-place arithmetic on the field forges it.
    // ctor / overwriteChecked (if==0) / overwriteAssert stay silent.
    assert_findings!(
        r,
        "E064" @ "takeWithoutClear",
        "E064" @ "takeViaReturn",
        "W012" @ "overwriteOnce",
        "W012" @ "overwriteLive",
        "W012" @ "overwriteLive",
        "E060" @ "mutate",
    );
}

#[test]
fn flow_owned_field_never_disposed() {
    // No method consumes the owned field -> W013, pinned to the leaked field.
    // (The constructor's store is exempt — the field starts at 0 — so no W012.)
    let leak = flow_report("example/OwnedFieldLeak.class");
    assert_findings!(leak, "W013" @ "handle");

    // The control class disposes its field via the safe swap-then-consume in
    // close(), so no W013; badClose() consumes before clearing -> E064.
    let ok = flow_report("example/OwnedFieldDisposed.class");
    assert_findings!(ok, "E064" @ "badClose");
}

#[test]
fn flow_exclusive_aliasing() {
    // assign(p, p) with assign(@Mut, @Ref) -> E065.
    let r = flow_report("example/AliasFlow.class");
    assert_findings!(r, "E065" @ "aliasMutRef");
}

#[test]
fn flow_affine_move() {
    // `b = a` moves a; using a afterwards -> E063.
    let r = flow_report("example/AffineMove.class");
    assert_findings!(r, "E063" @ "doubleUse");
}

#[test]
fn flow_exposed_handles() {
    // A public handle field (`exposed`) and a public handle-returning method
    // (`take`) each raise W014; the private `hidden` field stays silent.
    let r = flow_report("example/ExposeFlow.class");
    // W014 is declaration-level (one per exposed surface): the public field, the
    // public handle-returning method, and the public handle-taking methods.
    // `clone` exposes a handle on both its return and a parameter, so it raises
    // two. The private `hidden` field stays silent.
    assert_findings!(
        r,
        "W014" @ "exposed",
        "W014" @ "clone",
        "W014" @ "clone",
        "W014" @ "makeUppercase",
        "W014" @ "drop",
        "W014" @ "take",
    );
}

#[test]
fn flow_suppression_silences_category() {
    // Two identical E061 violations; @SuppressJni("transmute") silences the one
    // in `suppressed()`, leaving only `active()`.
    let r = flow_report("example/SuppressFlow.class");
    assert_findings!(r, "E061" @ "active");
}

#[test]
fn flow_lowers_bytecode() {
    // Phase 2 sanity: `load_flow` decodes method bodies into the owned model.
    // `Forge.fabricate` is `long p = 12345L; dropString(p);`.
    use jnisafe_check::code::Op;
    let classes = java_loader::load_flow(&[class("example/Forge.class")]).unwrap();
    let forge = classes
        .iter()
        .find(|c| c.internal_name == "example/Forge")
        .expect("Forge class");
    let fab = forge
        .methods
        .iter()
        .find(|m| m.name == "fabricate")
        .expect("fabricate method");
    let code = fab.code.as_ref().expect("fabricate has decoded bytecode");

    assert!(
        code.insns
            .iter()
            .any(|i| matches!(&i.op, Op::LongConst { zero: false })),
        "expected a non-zero long constant"
    );
    assert!(
        code.insns.iter().any(|i| matches!(&i.op, Op::StoreLong(_))),
        "expected an lstore"
    );
    assert!(
        code.insns.iter().any(|i| matches!(&i.op, Op::LoadLong(_))),
        "expected an lload"
    );
    assert!(
        code.insns.iter().any(
            |i| matches!(&i.op, Op::Invoke { target, is_static: true, .. } if target.name == "dropString")
        ),
        "expected an invokestatic of dropString"
    );
    // Line numbers are present (fixtures compile with `javac -g`).
    assert!(
        code.insns.iter().any(|i| i.line.is_some()),
        "expected a line-number mapping"
    );
}

// ---- JDK stdlib resolution (--java-home) ----------------------------------
//
// Binding to a JDK type (e.g. `java.nio.ByteBuffer`) that is not passed on
// `--java`: the checker resolves the class (and its supertype closure) from the
// JDK pointed to by `--java-home`/`$JAVA_HOME`. These tests skip when no full
// JDK is available (bare `cargo test`); `pixi run test` always provides one.

/// `load_jdk_models` resolves a referenced JDK class *and* its transitive
/// supertype closure, recording each class's superclass link.
#[test]
fn load_jdk_models_resolves_bytebuffer_and_supertypes() {
    let Some(home) = jdk_home() else {
        eprintln!("skipping: no JDK with jmods/ at $JAVA_HOME");
        return;
    };
    let seeds: HashSet<String> = ["java/nio/ByteBuffer".to_owned()].into_iter().collect();
    let models = java_loader::load_jdk_models(&home, &seeds, &HashSet::new()).expect("load jdk");

    let names: HashSet<&str> = models.iter().map(|m| m.internal_name.as_str()).collect();
    assert!(
        names.contains("java/nio/ByteBuffer"),
        "seed resolved: {names:?}"
    );
    assert!(
        names.contains("java/nio/Buffer"),
        "superclass closure: {names:?}"
    );
    assert!(
        names.contains("java/lang/Object"),
        "root of chain: {names:?}"
    );

    let bb = models
        .iter()
        .find(|m| m.internal_name == "java/nio/ByteBuffer")
        .unwrap();
    assert_eq!(bb.super_class.as_deref(), Some("java/nio/Buffer"));
}

/// A JDK home with no `jmods/` (and no `rt.jar`) resolves nothing rather than
/// erroring — the caller falls back to W004.
#[test]
fn load_jdk_models_without_jdk_source_is_graceful() {
    let seeds: HashSet<String> = ["java/nio/ByteBuffer".to_owned()].into_iter().collect();
    // The crate root is not a JDK: no jmods/, no rt.jar.
    let models = java_loader::load_jdk_models(&root(), &seeds, &HashSet::new()).expect("graceful");
    assert!(models.is_empty(), "no JDK source should resolve nothing");
}

/// `bind_java_type!` to members **declared on** `java.nio.ByteBuffer` verifies
/// cleanly once the class is resolved from the JDK (PR-A).
#[test]
fn jdk_declared_members_verify() {
    if jdk_home().is_none() {
        eprintln!("skipping: no JDK with jmods/ at $JAVA_HOME");
        return;
    }
    // No `--java` classes: the bound `java.nio.ByteBuffer` comes purely from the JDK.
    let cfg = config(fixture("tests/fixtures/jdk_declared"), vec![]);
    let report = run(&cfg).expect("run");
    assert_findings!(report);
}

/// A binding to a method that exists nowhere in ByteBuffer's hierarchy is a hard
/// E040 (the class is resolved, so it is *not* W004).
#[test]
fn jdk_missing_member_reports_e040() {
    if jdk_home().is_none() {
        eprintln!("skipping: no JDK with jmods/ at $JAVA_HOME");
        return;
    }
    let cfg = config(fixture("tests/fixtures/jdk_missing"), vec![]);
    let report = run(&cfg).expect("run");
    assert_findings!(report, "E040" @ "java.nio.ByteBuffer.ghostMethod");
}

/// `bind_java_type!` to members **inherited from `java.nio.Buffer`** verifies
/// cleanly only once member lookup walks the superclass chain (PR-B).
#[test]
fn jdk_inherited_members_verify() {
    if jdk_home().is_none() {
        eprintln!("skipping: no JDK with jmods/ at $JAVA_HOME");
        return;
    }
    let cfg = config(fixture("tests/fixtures/jdk_inherited"), vec![]);
    let report = run(&cfg).expect("run");
    assert_findings!(report);
}
