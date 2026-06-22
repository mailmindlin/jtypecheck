//! The matcher: pair Java native methods with Rust exports by mangled symbol,
//! then compare the two [`Signature`]s field by field.

use std::collections::{HashMap, HashSet};

use crate::diagnostics::{Diagnostic, Report};
use crate::ir::{IrType, PointerKind, Receiver, Signature};

/// Compare the Java and Rust sides and accumulate diagnostics.
pub fn check(java: &[Signature], rust: &[Signature]) -> Report {
    let mut report = Report::default();

    // Index Rust exports by symbol; flag duplicates.
    let mut rust_index: HashMap<&str, &Signature> = HashMap::new();
    for r in rust {
        if rust_index.insert(&r.key.symbol, r).is_some() {
            report.push(
                Diagnostic::error("E000", format!("duplicate Rust export `{}`", r.key.symbol))
                    .with_rust(r.origin.rust.clone()),
            );
        }
    }

    let mut matched: HashSet<&str> = HashSet::new();

    for j in java {
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

    // Orphan Rust exports: `Java_*` with no Java native method.
    for r in rust {
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
    }

    report
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
