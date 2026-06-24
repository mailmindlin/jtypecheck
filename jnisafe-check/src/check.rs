//! The matcher: pair Java native methods with Rust exports by mangled symbol,
//! then compare the two [`Signature`]s field by field.

use std::collections::{HashMap, HashSet};

use crate::diagnostics::{Diagnostic, Report};
use crate::ir::{IrType, PointerKind, Receiver, RustExportProblem, Signature};

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
    use super::check;
    use crate::ir::{IrType, JavaLoc, MethodKey, Origin, Receiver, Signature};

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

    #[test]
    fn distinct_java_symbols_no_e005() {
        let java = vec![
            java_sig("Java_example_Foo_a", "a"),
            java_sig("Java_example_Foo_b", "b"),
        ];
        assert!(!check(&java, &[]).has_code("E005"));
    }
}
