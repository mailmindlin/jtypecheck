//! The matcher: pair Java native methods with Rust exports by mangled symbol,
//! then compare the two [`Signature`]s field by field.

use std::collections::{HashMap, HashSet};

use crate::diagnostics::{Diagnostic, Report};
use crate::ir::{
    IrType, JavaClassModel, JavaFieldSig, JavaMethodSig, JavaRef, JavaRefKind, Pointer,
    PointerKind, Receiver, RustExportProblem, Signature, args_descriptor,
};

/// Compare the Java and Rust sides and accumulate diagnostics.
pub fn check(java: &[Signature], rust: &[Signature]) -> Report {
    let mut report = Report::default();

    // Index Rust exports by symbol; flag duplicates. Structurally-broken
    // `Java_*` fns (W002 / E004) are reported here and excluded from matching
    // and from the orphan check below.
    let mut rust_index: HashMap<&str, &Signature> = HashMap::new();
    for r in rust {
        if let Some(problem) = r.export_problem {
            report.push(export_problem_diag(r, problem));
            continue;
        }
        if rust_index.insert(&r.key.symbol, r).is_some() {
            report.push(
                Diagnostic::error("E000", format!("duplicate Rust export `{}`", r.key.symbol))
                    .with_rust(r.origin.rust.clone()),
            );
        }
    }

    let mut matched: HashSet<&str> = HashSet::new();
    let mut java_seen: HashSet<&str> = HashSet::new();

    for j in java {
        // Two Java natives mangling to one symbol (E005) — the mirror of the
        // Rust-side E000 duplicate check.
        if !java_seen.insert(j.key.symbol.as_str()) {
            report.push(
                Diagnostic::error(
                    "E005",
                    format!(
                        "duplicate Java native methods mangle to the same symbol `{}`",
                        j.key.symbol
                    ),
                )
                .with_java(j.origin.java.clone())
                .note("two native declarations produce one JNI symbol; only one Rust export can match"),
            );
        }

        let Some(r) = rust_index.get(j.key.symbol.as_str()).copied() else {
            report.push(
                Diagnostic::error(
                    "E001",
                    format!(
                        "no Rust export found for native method {}.{}",
                        j.key.java_class.replace('/', "."),
                        j.key.java_method
                    ),
                )
                .with_java(j.origin.java.clone())
                .note(format!("expected exported symbol `{}`", j.key.symbol))
                .help(format!(
                    "define `#[no_mangle] pub extern \"system\" fn {}(..)` or annotate the method @io.github.mailmindlin.jnisafe.Ignore",
                    j.key.symbol
                )),
            );
            continue;
        };
        matched.insert(j.key.symbol.as_str());

        // Receiver: static ↔ JClass, instance ↔ JObject.
        check_receiver(j, r, &mut report);

        // Arity (Java params vs Rust params after env+receiver were skipped).
        if j.params.len() != r.params.len() {
            report.push(
                Diagnostic::error(
                    "E010",
                    format!(
                        "parameter count mismatch for {}.{}",
                        j.key.java_class.replace('/', "."),
                        j.key.java_method
                    ),
                )
                .with_java(j.origin.java.clone())
                .with_rust(r.origin.rust.clone())
                .expected_found(
                    format!("{} parameter(s) (Java)", j.params.len()),
                    format!(
                        "{} parameter(s) (Rust, after JNIEnv + class/this)",
                        r.params.len()
                    ),
                ),
            );
            continue;
        }

        for (i, (jp, rp)) in j.params.iter().zip(&r.params).enumerate() {
            for d in compare(jp, rp, "E02", &format!("parameter {i}"), j, r) {
                report.push(d);
            }
        }
        for d in compare(&j.ret, &r.ret, "E03", "return type", j, r) {
            report.push(d);
        }
    }

    // Per-Rust-export lints over well-formed exports: orphans (W001) and
    // borrow-handle returns (W003). Broken exports already reported above.
    for r in rust {
        if r.export_problem.is_some() {
            continue;
        }
        if !matched.contains(&r.key.symbol.as_str()) {
            report.push(
                Diagnostic::warning(
                    "W001",
                    format!(
                        "orphan Rust export `{}` (no matching Java native method)",
                        r.key.symbol
                    ),
                )
                .with_rust(r.origin.rust.clone()),
            );
        }
        // Returning a borrow handle hands a borrowed lifetime to Java, which has
        // no borrow checker to keep the pointee alive (W003).
        if let IrType::Pointer(p) = &r.ret
            && matches!(p.kind, PointerKind::Ref | PointerKind::Mut)
        {
            report.push(
                Diagnostic::warning(
                    "W003",
                    format!("Rust export `{}` returns a borrow handle", r.key.symbol),
                )
                .with_rust(r.origin.rust.clone())
                .expected_found(
                    "an owned return (`JOwned`, an object, or a primitive)",
                    format!("a borrowed `{}` handle", p.kind.wrapper()),
                )
                .help(
                    "return `JOwned` instead; a borrowed `JRef`/`JMut` outlives its Rust borrow once handed to Java → use-after-free",
                ),
            );
        }
    }

    report
}

/// Verify the Rust→Java call bindings declared by `bind_java_type!`'s
/// `methods`/`fields`/`constructors` clauses: each names a Java member the Rust
/// side intends to *call*, so we confirm it exists in the loaded class with a
/// matching receiver and JVM descriptor. (Java→Rust natives are handled by
/// [`check`]; this is the reverse direction.)
pub fn check_java_refs(refs: &[JavaRef], models: &[JavaClassModel], report: &mut Report) {
    let index: HashMap<&str, &JavaClassModel> = models
        .iter()
        .map(|m| (m.internal_name.as_str(), m))
        .collect();

    for r in refs {
        let Some(model) = index.get(r.class_internal.as_str()).copied() else {
            report.push(
                Diagnostic::warning(
                    "W004",
                    format!(
                        "cannot verify {} `{}`: Java class `{}` was not provided to --java",
                        r.kind.noun(),
                        r.java_name,
                        r.class_internal.replace('/', ".")
                    ),
                )
                .with_rust(Some(r.origin.clone()))
                .help(
                    "pass that class (or its containing dir/jar) on --java to check this binding",
                ),
            );
            continue;
        };
        match r.kind {
            JavaRefKind::Method => check_method_ref(r, model, report),
            JavaRefKind::Constructor => check_ctor_ref(r, model, report),
            JavaRefKind::Field => check_field_ref(r, model, report),
        }
    }
}

/// The expected return descriptor for a `JavaRef` method (`"V"` for void).
fn expected_ret(ret: &IrType) -> Option<String> {
    match ret {
        IrType::Void => Some("V".to_owned()),
        other => other.jni_field_descriptor(),
    }
}

fn check_method_ref(r: &JavaRef, model: &JavaClassModel, report: &mut Report) {
    let class = r.class_internal.replace('/', ".");
    let by_name: Vec<&JavaMethodSig> = model
        .methods
        .iter()
        .filter(|m| m.name == r.java_name)
        .collect();
    if by_name.is_empty() {
        report.push(
            Diagnostic::error(
                "E040",
                format!("no Java method `{}` on `{class}`", r.java_name),
            )
            .with_rust(Some(r.origin.clone()))
            .help("check the method name (Rust `snake_case` maps to Java `lowerCamelCase`) or `name = \"…\"`"),
        );
        return;
    }
    let by_receiver: Vec<&&JavaMethodSig> = by_name
        .iter()
        .filter(|m| m.is_static == r.is_static)
        .collect();
    if by_receiver.is_empty() {
        report.push(
            Diagnostic::error(
                "E040",
                format!(
                    "no {} Java method `{}` on `{class}`",
                    receiver_word(r.is_static),
                    r.java_name
                ),
            )
            .with_rust(Some(r.origin.clone()))
            .note(format!(
                "found a {} method of the same name — fix the `static` qualifier",
                receiver_word(!r.is_static)
            )),
        );
        return;
    }

    // Compare signatures only if the Rust types are encodable (unsupported types
    // are flagged elsewhere); the name+receiver match already holds.
    let (Some(eargs), Some(eret)) = (args_descriptor(&r.params), expected_ret(&r.ret)) else {
        return;
    };
    if by_receiver
        .iter()
        .any(|m| m.arg_descriptor == eargs && m.ret_descriptor == eret)
    {
        return;
    }
    let found = by_receiver
        .iter()
        .map(|m| format!("({}){}", m.arg_descriptor, m.ret_descriptor))
        .collect::<Vec<_>>()
        .join(" | ");
    report.push(
        Diagnostic::error(
            "E041",
            format!(
                "Java method `{}` on `{class}` has no matching signature",
                r.java_name
            ),
        )
        .with_rust(Some(r.origin.clone()))
        .expected_found(format!("({eargs}){eret}"), found),
    );
}

fn check_ctor_ref(r: &JavaRef, model: &JavaClassModel, report: &mut Report) {
    let class = r.class_internal.replace('/', ".");
    let Some(eargs) = args_descriptor(&r.params) else {
        return;
    };
    if model.constructors.contains(&eargs) {
        return;
    }
    let found = if model.constructors.is_empty() {
        "no constructors".to_owned()
    } else {
        model
            .constructors
            .iter()
            .map(|c| format!("({c})V"))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    report.push(
        Diagnostic::error("E044", format!("no constructor `({eargs})V` on `{class}`"))
            .with_rust(Some(r.origin.clone()))
            .expected_found(format!("({eargs})V"), found),
    );
}

fn check_field_ref(r: &JavaRef, model: &JavaClassModel, report: &mut Report) {
    let class = r.class_internal.replace('/', ".");
    let by_name: Vec<&JavaFieldSig> = model
        .fields
        .iter()
        .filter(|f| f.name == r.java_name)
        .collect();
    if by_name.is_empty() {
        report.push(
            Diagnostic::error("E042", format!("no Java field `{}` on `{class}`", r.java_name))
                .with_rust(Some(r.origin.clone()))
                .help("check the field name (Rust `snake_case` maps to Java `lowerCamelCase`) or `name = \"…\"`"),
        );
        return;
    }
    let by_receiver: Vec<&&JavaFieldSig> = by_name
        .iter()
        .filter(|f| f.is_static == r.is_static)
        .collect();
    if by_receiver.is_empty() {
        report.push(
            Diagnostic::error(
                "E042",
                format!(
                    "no {} Java field `{}` on `{class}`",
                    receiver_word(r.is_static),
                    r.java_name
                ),
            )
            .with_rust(Some(r.origin.clone()))
            .note(format!(
                "found a {} field of the same name — fix the `static` qualifier",
                receiver_word(!r.is_static)
            )),
        );
        return;
    }
    let Some(expected) = r.field_ty.as_ref().and_then(|t| t.jni_field_descriptor()) else {
        return;
    };
    let matching: Vec<&JavaFieldSig> = by_receiver
        .iter()
        .filter(|f| f.descriptor == expected)
        .map(|f| **f)
        .collect();
    if matching.is_empty() {
        let found = by_receiver
            .iter()
            .map(|f| f.descriptor.clone())
            .collect::<Vec<_>>()
            .join(" | ");
        report.push(
            Diagnostic::error(
                "E043",
                format!(
                    "Java field `{}` on `{class}` has the wrong type",
                    r.java_name
                ),
            )
            .with_rust(Some(r.origin.clone()))
            .expected_found(expected, found),
        );
        return;
    }
    // The descriptor matches (both `long`). If the Rust side declares this field
    // as a handle, a bare `long` is indistinguishable from any other handle on
    // the wire, so the Java field must carry a matching `@Ref`/`@Mut`/`@Owned`
    // annotation for the stored type to be checkable.
    if let Some(IrType::Pointer(rust_ptr)) = r.field_ty.as_ref() {
        check_field_handle_annotation(r, rust_ptr, &matching, &class, report);
    }
}

/// Cross-check a Rust handle-typed field declaration against the `@Ref`/`@Mut`/
/// `@Owned` annotation on the matching Java `long` field(s): a clean match if any
/// agrees, **W005** if none is annotated, **E045** if an annotation is present
/// but disagrees on kind/type (or nullability, for the borrow handles).
fn check_field_handle_annotation(
    r: &JavaRef,
    rust_ptr: &Pointer,
    matching: &[&JavaFieldSig],
    class: &str,
    report: &mut Report,
) {
    if matching
        .iter()
        .filter_map(|f| f.annotation.as_ref())
        .any(|java_ptr| pointer_matches(rust_ptr, java_ptr))
    {
        return;
    }
    let annotated: Vec<&Pointer> = matching
        .iter()
        .filter_map(|f| f.annotation.as_ref())
        .collect();
    let expected = IrType::Pointer(rust_ptr.clone()).describe();
    if annotated.is_empty() {
        report.push(
            Diagnostic::warning(
                "W005",
                format!(
                    "Java field `{}` on `{class}` stores a `{}` handle but is not annotated",
                    r.java_name,
                    rust_ptr.kind.wrapper()
                ),
            )
            .with_rust(Some(r.origin.clone()))
            .help(format!(
                "annotate the `long` field `{}(\"{}\")` so the stored handle type is cross-checked",
                rust_ptr.kind.annotation(),
                rust_ptr.rust_type
            )),
        );
        return;
    }
    let found = annotated
        .iter()
        .map(|p| IrType::Pointer((*p).clone()).describe())
        .collect::<Vec<_>>()
        .join(" | ");
    report.push(
        Diagnostic::error(
            "E045",
            format!(
                "Java field `{}` on `{class}` has a mismatched handle annotation",
                r.java_name
            ),
        )
        .with_rust(Some(r.origin.clone()))
        .expected_found(expected, found)
        .help("make the Java `@Ref`/`@Mut`/`@Owned` annotation match the Rust handle type"),
    );
}

/// Whether a Rust handle field type agrees with a Java field annotation. Mirrors
/// the parameter-path rule in [`compare`]: kind and pointee type must match, and
/// nullability is compared only for the borrow handles (`@Ref`/`@Mut`) — `JOwned`
/// is internally nullable and never `Option`-wrapped, so its nullability is moot.
fn pointer_matches(rust: &Pointer, java: &Pointer) -> bool {
    if rust.kind != java.kind || rust.rust_type != java.rust_type {
        return false;
    }
    match rust.kind {
        PointerKind::Ref | PointerKind::Mut => rust.nullable == java.nullable,
        PointerKind::Owned => true,
    }
}

fn receiver_word(is_static: bool) -> &'static str {
    if is_static { "static" } else { "instance" }
}

/// Diagnostic for a structurally-broken Rust `Java_*` fn (W002 / E004).
fn export_problem_diag(r: &Signature, problem: RustExportProblem) -> Diagnostic {
    match problem {
        RustExportProblem::NotExported => Diagnostic::warning(
            "W002",
            format!("`{}` looks like a JNI export but is not exported", r.key.symbol),
        )
        .with_rust(r.origin.rust.clone())
        .help(
            "add `#[unsafe(no_mangle)]` and `extern \"system\"`, or rename it if it is not a native method",
        ),
        RustExportProblem::TooFewParams => Diagnostic::error(
            "E004",
            format!(
                "JNI export `{}` has too few parameters (no room for JNIEnv + class/this)",
                r.key.symbol
            ),
        )
        .with_rust(r.origin.rust.clone())
        .help(
            "a native method's Rust fn takes `JNIEnv`/`EnvUnowned` then a `JClass`/`JObject` receiver, before its declared parameters",
        ),
    }
}

fn check_receiver(j: &Signature, r: &Signature, report: &mut Report) {
    let mismatch = match (j.is_static, r.receiver) {
        (true, Receiver::Object) => Some(("static Java method", "JObject", "JClass")),
        (false, Receiver::Class) => Some(("instance Java method", "JClass", "JObject")),
        _ => None,
    };
    if let Some((what, got, want)) = mismatch {
        report.push(
            Diagnostic::error(
                "E002",
                format!(
                    "receiver mismatch for {}.{}: {what} mapped to Rust `{got}`",
                    j.key.java_class.replace('/', "."),
                    j.key.java_method
                ),
            )
            .with_java(j.origin.java.clone())
            .with_rust(r.origin.rust.clone())
            .help(format!(
                "expected the Rust fn's second parameter to be `{want}`"
            )),
        );
    }
}

/// Compare one Java type against one Rust type, emitting field-level
/// diagnostics. `prefix` is `E02` (params) or `E03` (return); the last digit
/// encodes the dimension of the mismatch.
fn compare(
    java: &IrType,
    rust: &IrType,
    prefix: &str,
    ctx: &str,
    j: &Signature,
    r: &Signature,
) -> Vec<Diagnostic> {
    let mk = |digit: char, msg: String| -> Diagnostic {
        Diagnostic::error(format!("{prefix}{digit}"), msg)
            .with_java(j.origin.java.clone())
            .with_rust(r.origin.rust.clone())
    };
    let head = format!(
        "{ctx} of {}.{}",
        j.key.java_class.replace('/', "."),
        j.key.java_method
    );

    // A pointer annotation on a non-`long` slot (E026), reported once and
    // short-circuited. Only the Java loader produces `Misannotated`.
    if let IrType::Misannotated {
        ann_kind,
        java_desc,
        narrow_int,
    } = java
    {
        let msg = if *narrow_int {
            format!(
                "{head}: {} annotation on `{java_desc}` — a JNI handle is a 64-bit `long` and does not fit (it would be truncated)",
                ann_kind.annotation()
            )
        } else {
            format!(
                "{head}: {} annotation on a non-`long` slot (`{java_desc}`)",
                ann_kind.annotation()
            )
        };
        return vec![
            mk('6', msg)
                .help("place `@Ref`/`@Mut`/`@Owned` only on a `long` parameter or return type"),
        ];
    }

    // An unsupported type on either side is reported once and short-circuits.
    if let IrType::Unsupported(s) = java {
        return vec![mk('9', format!("{head}: unsupported Java type `{s}`"))];
    }
    if let IrType::Unsupported(s) = rust {
        return vec![mk('9', format!("{head}: unsupported Rust type `{s}`"))];
    }

    match (java, rust) {
        (IrType::Void, IrType::Void) => vec![],
        (IrType::Primitive(a), IrType::Primitive(b)) => {
            if a == b {
                vec![]
            } else {
                vec![
                    mk('1', format!("{head}: primitive mismatch"))
                        .expected_found(java.describe(), rust.describe()),
                ]
            }
        }
        (IrType::JavaObject { class: a }, IrType::JavaObject { class: b }) => {
            if a == b {
                vec![]
            } else {
                vec![
                    mk('2', format!("{head}: Java object type mismatch"))
                        .expected_found(java.describe(), rust.describe()),
                ]
            }
        }
        (IrType::Pointer(a), IrType::Pointer(b)) => {
            let mut out = Vec::new();
            if a.kind != b.kind {
                out.push(
                    mk('3', format!("{head}: pointer kind mismatch"))
                        .expected_found(
                            format!("{} (Rust `{}`)", a.kind.annotation(), a.kind.wrapper()),
                            format!("{} (Rust `{}`)", b.kind.annotation(), b.kind.wrapper()),
                        )
                        .help("change the Java annotation or the Rust wrapper so the kinds agree"),
                );
            }
            if a.rust_type != b.rust_type {
                out.push(
                    mk('4', format!("{head}: pointer type mismatch"))
                        .expected_found(a.rust_type.clone(), b.rust_type.clone()),
                );
            }
            // Enforce the nullability ↔ `Option` convention for the borrow
            // handles (`@Ref`/`@Mut`). A bare `JRef`/`JMut` is
            // `#[repr(transparent)]` over `NonZero<jlong>`, so a null (`0`)
            // materializing into one is immediate UB on entry — unrecoverable,
            // not even runtime-checkable. Requiring nullable annotations to use
            // `Option<..>` (which decodes `0` to `None` via the niche) is the
            // only static guard available. `@Owned` is excluded: `JOwned` is
            // internally nullable (it must yield `Default` = null on the error
            // path), so it is never wrapped in `Option` and has no niche to
            // protect.
            let both_borrow =
                a.kind == b.kind && matches!(a.kind, PointerKind::Ref | PointerKind::Mut);
            if both_borrow && a.nullable != b.nullable {
                out.push(
                    mk('5', format!("{head}: nullability mismatch"))
                        .expected_found(
                            nullability_desc(a.nullable),
                            nullability_desc(b.nullable),
                        )
                        .help("a nullable Java annotation maps to Rust `Option<JRef<..>>`; non-nullable maps to a bare wrapper"),
                );
            }
            out
        }
        _ => vec![
            mk('0', format!("{head}: type category mismatch"))
                .expected_found(java.describe(), rust.describe()),
        ],
    }
}

fn nullability_desc(nullable: bool) -> String {
    if nullable {
        "nullable (Java default / Rust `Option<..>`)".to_owned()
    } else {
        "non-nullable (Java `nullable=false` / bare Rust wrapper)".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{check, check_java_refs};
    use crate::diagnostics::Report;
    use crate::ir::{
        IrType, JavaClassModel, JavaFieldSig, JavaLoc, JavaMethodSig, JavaRef, JavaRefKind,
        MethodKey, Origin, Pointer, PointerKind, Primitive, Receiver, Signature, SrcLoc,
    };
    use std::path::PathBuf;

    fn java_sig(symbol: &str, method: &str) -> Signature {
        Signature {
            key: MethodKey {
                symbol: symbol.to_owned(),
                java_class: "example/Foo".to_owned(),
                java_method: method.to_owned(),
            },
            is_static: true,
            receiver: Receiver::Unknown,
            params: Vec::new(),
            ret: IrType::Void,
            origin: Origin {
                rust: None,
                java: Some(JavaLoc {
                    class: "example/Foo".to_owned(),
                    method: method.to_owned(),
                    descriptor: "()V".to_owned(),
                }),
            },
            export_problem: None,
        }
    }

    #[test]
    fn duplicate_java_symbol_reports_e005() {
        // Two distinct natives mangling to one symbol (hard to express via javac,
        // which forbids true duplicates).
        let java = vec![
            java_sig("Java_example_Foo_bar", "bar"),
            java_sig("Java_example_Foo_bar", "bar"),
        ];
        let report = check(&java, &[]);
        assert!(
            report.has_code("E005"),
            "expected E005:\n{}",
            report.render_human()
        );
    }

    // --- Rust→Java reference checks ----------------------------------------

    fn int() -> IrType {
        IrType::Primitive(Primitive::Int)
    }

    fn model() -> JavaClassModel {
        JavaClassModel {
            internal_name: "example/Foo".to_owned(),
            methods: vec![JavaMethodSig {
                name: "doubled".to_owned(),
                is_static: true,
                arg_descriptor: "I".to_owned(),
                ret_descriptor: "I".to_owned(),
            }],
            fields: vec![JavaFieldSig {
                name: "counter".to_owned(),
                is_static: true,
                descriptor: "I".to_owned(),
                annotation: None,
            }],
            constructors: vec!["I".to_owned()],
        }
    }

    fn ref_(
        kind: JavaRefKind,
        name: &str,
        is_static: bool,
        params: Vec<IrType>,
        ret: IrType,
        field_ty: Option<IrType>,
    ) -> JavaRef {
        JavaRef {
            class_internal: "example/Foo".to_owned(),
            kind,
            java_name: name.to_owned(),
            is_static,
            params,
            ret,
            field_ty,
            origin: SrcLoc {
                file: PathBuf::from("lib.rs"),
                symbol: format!("example.Foo.{name}"),
                line: Some(1),
            },
        }
    }

    fn run_refs(refs: &[JavaRef]) -> Report {
        let mut report = Report::default();
        check_java_refs(refs, &[model()], &mut report);
        report
    }

    #[test]
    fn matching_refs_clean() {
        let refs = vec![
            ref_(
                JavaRefKind::Method,
                "doubled",
                true,
                vec![int()],
                int(),
                None,
            ),
            ref_(
                JavaRefKind::Field,
                "counter",
                true,
                vec![],
                IrType::Void,
                Some(int()),
            ),
            ref_(
                JavaRefKind::Constructor,
                "<init>",
                false,
                vec![int()],
                IrType::Void,
                None,
            ),
        ];
        let report = run_refs(&refs);
        assert!(
            !report.has_errors(),
            "expected clean:\n{}",
            report.render_human()
        );
        assert_eq!(report.diagnostics.len(), 0);
    }

    #[test]
    fn missing_method_is_e040_and_bad_signature_is_e041() {
        let refs = vec![
            ref_(
                JavaRefKind::Method,
                "tripled",
                true,
                vec![int()],
                int(),
                None,
            ),
            ref_(
                JavaRefKind::Method,
                "doubled",
                true,
                vec![int(), int()],
                int(),
                None,
            ),
        ];
        let report = run_refs(&refs);
        assert!(report.has_code("E040"), "{}", report.render_human());
        assert!(report.has_code("E041"), "{}", report.render_human());
    }

    #[test]
    fn field_and_constructor_problems() {
        let refs = vec![
            ref_(
                JavaRefKind::Field,
                "missing",
                true,
                vec![],
                IrType::Void,
                Some(int()),
            ),
            ref_(
                JavaRefKind::Field,
                "counter",
                true,
                vec![],
                IrType::Void,
                Some(IrType::JavaObject {
                    class: "java/lang/String".to_owned(),
                }),
            ),
            ref_(
                JavaRefKind::Constructor,
                "<init>",
                false,
                vec![int(), int()],
                IrType::Void,
                None,
            ),
        ];
        let report = run_refs(&refs);
        assert!(report.has_code("E042"), "{}", report.render_human()); // missing field
        assert!(report.has_code("E043"), "{}", report.render_human()); // wrong field type
        assert!(report.has_code("E044"), "{}", report.render_human()); // no such constructor
    }

    #[test]
    fn unloaded_class_is_w004() {
        let mut r = ref_(
            JavaRefKind::Method,
            "doubled",
            true,
            vec![int()],
            int(),
            None,
        );
        r.class_internal = "example/NotLoaded".to_owned();
        let report = run_refs(&[r]);
        assert!(report.has_code("W004"), "{}", report.render_human());
        assert!(!report.has_errors());
    }

    #[test]
    fn distinct_java_symbols_no_e005() {
        let java = vec![
            java_sig("Java_example_Foo_a", "a"),
            java_sig("Java_example_Foo_b", "b"),
        ];
        assert!(!check(&java, &[]).has_code("E005"));
    }

    // --- Field handle-annotation cross-check (Rust handle field ↔ Java @Owned/…) -

    fn owned(rust_type: &str, nullable: bool) -> Pointer {
        Pointer {
            kind: PointerKind::Owned,
            rust_type: rust_type.to_owned(),
            nullable,
        }
    }

    /// A class with a single instance `long handle` field carrying `annotation`.
    fn handle_model(annotation: Option<Pointer>) -> JavaClassModel {
        JavaClassModel {
            internal_name: "example/Foo".to_owned(),
            methods: vec![],
            fields: vec![JavaFieldSig {
                name: "handle".to_owned(),
                is_static: false,
                descriptor: "J".to_owned(),
                annotation,
            }],
            constructors: vec![],
        }
    }

    fn run_refs_against(refs: &[JavaRef], model: JavaClassModel) -> Report {
        let mut report = Report::default();
        check_java_refs(refs, &[model], &mut report);
        report
    }

    fn handle_field_ref() -> JavaRef {
        ref_(
            JavaRefKind::Field,
            "handle",
            false,
            vec![],
            IrType::Void,
            Some(IrType::Pointer(owned("Box<String>", false))),
        )
    }

    #[test]
    fn handle_field_matching_annotation_is_clean() {
        // `@Owned` nullability is intentionally not compared: Java's default
        // `nullable = true` must still match the bare Rust `JOwned`.
        let report = run_refs_against(
            &[handle_field_ref()],
            handle_model(Some(owned("Box<String>", true))),
        );
        assert!(
            !report.has_errors(),
            "expected clean:\n{}",
            report.render_human()
        );
        assert_eq!(report.diagnostics.len(), 0);
    }

    #[test]
    fn unannotated_handle_field_is_w005() {
        let report = run_refs_against(&[handle_field_ref()], handle_model(None));
        assert!(report.has_code("W005"), "{}", report.render_human());
        assert!(!report.has_errors(), "W005 is a warning");
    }

    #[test]
    fn mismatched_handle_annotation_is_e045() {
        // Java declares the wrong pointee type.
        let report = run_refs_against(
            &[handle_field_ref()],
            handle_model(Some(owned("Box<u64>", true))),
        );
        assert!(report.has_code("E045"), "{}", report.render_human());
        assert!(report.has_errors());
    }

    #[test]
    fn mismatched_handle_kind_is_e045() {
        // Java annotates the handle field `@Ref` where Rust stores a `JOwned`.
        let java_ref = Pointer {
            kind: PointerKind::Ref,
            rust_type: "Box<String>".to_owned(),
            nullable: false,
        };
        let report = run_refs_against(&[handle_field_ref()], handle_model(Some(java_ref)));
        assert!(report.has_code("E045"), "{}", report.render_human());
    }
}
