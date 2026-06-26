//! Rust front-end: walk a crate's source with `syn`, find exported JNI
//! functions (`#[no_mangle] extern "system" fn Java_*`), and lower each into a
//! [`Signature`]. Hidden behind the [`RustExtractor`] trait so a future
//! rust-analyzer backend can replace it without touching the matcher.

use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::{FnArg, GenericArgument, Item, PathArguments, ReturnType, Type};

use crate::ir::{
    IrType, JavaRef, MethodKey, Origin, Pointer, PointerKind, Receiver, RustExportProblem,
    Signature, SrcLoc,
};
use crate::typemap;

mod macros;

/// What the Rust front-end extracts from a crate: native-method signatures
/// (Java→Rust, matched by symbol) plus the Rust→Java call bindings declared by
/// `bind_java_type!`'s `methods`/`fields`/`constructors` clauses.
#[derive(Debug, Default)]
pub struct RustArtifacts {
    pub natives: Vec<Signature>,
    pub java_refs: Vec<JavaRef>,
}

#[derive(Debug, thiserror::Error)]
pub enum RustLoadError {
    #[error("failed to read `{0}`: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("failed to parse `{0}`: {1}")]
    Parse(PathBuf, syn::Error),
}

/// Abstraction over "give me the JNI artifacts of a crate."
pub trait RustExtractor {
    fn extract(&self, crate_dir: &Path) -> Result<RustArtifacts, RustLoadError>;
}

/// Default `syn`-based source extractor.
pub struct SynBackend;

impl RustExtractor for SynBackend {
    fn extract(&self, crate_dir: &Path) -> Result<RustArtifacts, RustLoadError> {
        let mut arts = RustArtifacts::default();
        // `src/**/*.rs` if a crate root; otherwise treat the path itself / its
        // tree as the source set (lets tests point at a single fixture file).
        let root = {
            let src = crate_dir.join("src");
            if src.is_dir() {
                src
            } else {
                crate_dir.to_path_buf()
            }
        };
        if root.is_file() {
            extract_file(&root, &mut arts)?;
        } else {
            for entry in walkdir::WalkDir::new(&root).into_iter().flatten() {
                if entry.path().extension().is_some_and(|e| e == "rs") {
                    extract_file(entry.path(), &mut arts)?;
                }
            }
        }
        resolve_overloads(&mut arts.natives);
        Ok(arts)
    }
}

/// Re-mangle overloaded exports to the long `..._method__<args>` form, matching
/// how `java_loader` mangles a class that declares two natives of the same name.
/// Scoped to signatures whose symbol *we* computed (a non-empty `java_class`,
/// set by the `#[jni_mangle]` and macro paths); a legacy `Java_*` export carries
/// the user's verbatim symbol (empty `java_class`) and is left untouched.
fn resolve_overloads(sigs: &mut [Signature]) {
    use std::collections::HashMap;

    let mut groups: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
    for (i, s) in sigs.iter().enumerate() {
        if s.key.java_class.is_empty() {
            continue;
        }
        groups
            .entry((s.key.java_class.as_str(), s.key.java_method.as_str()))
            .or_default()
            .push(i);
    }
    // Collect the indices needing the long form first; `groups` borrows `sigs`,
    // so it must be dropped before we mutate.
    let overloaded: Vec<usize> = groups
        .into_values()
        .filter(|idxs| idxs.len() > 1)
        .flatten()
        .collect();

    for i in overloaded {
        // An unencodable param (e.g. an unsupported type) leaves the short form;
        // the type itself is already flagged by the matcher.
        let Some(desc) = crate::ir::args_descriptor(&sigs[i].params) else {
            continue;
        };
        let symbol = crate::mangle::mangle(
            &sigs[i].key.java_class,
            &sigs[i].key.java_method,
            true,
            &desc,
        );
        sigs[i].key.symbol = symbol.clone();
        if let Some(rust) = sigs[i].origin.rust.as_mut() {
            rust.symbol = symbol;
        }
    }
}

fn extract_file(path: &Path, arts: &mut RustArtifacts) -> Result<(), RustLoadError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| RustLoadError::Io(path.to_path_buf(), e))?;
    let file =
        syn::parse_file(&content).map_err(|e| RustLoadError::Parse(path.to_path_buf(), e))?;
    for item in &file.items {
        let Item::Fn(f) = item else { continue };
        if let Some(sig) = lower_fn(f, path) {
            arts.natives.push(sig);
        }
    }
    // Also recognize the jni 0.22.4 macros: native methods (`native_method! { }`,
    // `bind_java_type! { native_methods { } }`) and the Rust→Java call bindings
    // (`bind_java_type!`'s `methods`/`fields`/`constructors`).
    macros::collect(&file, path, &mut arts.natives, &mut arts.java_refs);
    Ok(())
}

fn lower_fn(f: &syn::ItemFn, path: &Path) -> Option<Signature> {
    let ident = f.sig.ident.to_string();

    // Two recognized fn shapes: a `#[jni_mangle("pkg.Class")]`-attributed fn
    // (any name — the macro derives the export symbol) or the legacy
    // `Java_*`-named export. Anything else is not a native method.
    let mangle = parse_jni_mangle(&f.attrs);
    if mangle.is_none() && !ident.starts_with("Java_") {
        return None;
    }

    // Inputs: skip the JNIEnv and the receiver (JClass / JObject), but read the
    // receiver type so we can cross-check static-vs-instance against Java.
    let inputs: Vec<&Type> = f
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(pt) => Some(&*pt.ty),
            FnArg::Receiver(_) => None,
        })
        .collect();

    let receiver = inputs
        .get(1)
        .map(|t| receiver_kind(t))
        .unwrap_or(Receiver::Unknown);
    let params: Vec<IrType> = inputs.iter().skip(2).map(|t| lower_type(t)).collect();
    let ret = match &f.sig.output {
        ReturnType::Default => IrType::Void,
        ReturnType::Type(_, ty) => lower_type(ty),
    };

    let (key, export_problem) = match &mangle {
        Some(m) => {
            // `#[jni_mangle]` derives the export symbol and ABI itself, so the
            // only structural problem worth flagging is too few params for the
            // env + receiver every JNI entry point needs.
            let class_internal = crate::mangle::class_dotted_to_internal(&m.namespace);
            let method = m
                .method_name
                .clone()
                .unwrap_or_else(|| crate::mangle::snake_to_lower_camel(&ident));
            let symbol = match &m.arg_descriptor {
                Some(desc) => crate::mangle::mangle(&class_internal, &method, true, desc),
                None => crate::mangle::mangle(&class_internal, &method, false, ""),
            };
            let problem = (inputs.len() < 2).then_some(RustExportProblem::TooFewParams);
            (
                MethodKey {
                    symbol,
                    java_class: class_internal,
                    java_method: method,
                },
                problem,
            )
        }
        None => {
            // A `Java_*`-named fn is meant to be an export; lower it even when it
            // isn't a valid one so the checker can flag the mistake (W002 / E004)
            // instead of silently dropping it.
            let problem = if !is_no_mangle(&f.attrs) || !is_system_abi(&f.sig.abi) {
                Some(RustExportProblem::NotExported)
            } else if inputs.len() < 2 {
                Some(RustExportProblem::TooFewParams)
            } else {
                None
            };
            (
                MethodKey {
                    symbol: ident.clone(),
                    java_class: String::new(),
                    java_method: String::new(),
                },
                problem,
            )
        }
    };

    let symbol = key.symbol.clone();
    Some(Signature {
        key,
        is_static: false,
        receiver,
        params,
        ret,
        origin: Origin {
            rust: Some(SrcLoc {
                file: path.to_path_buf(),
                symbol,
                line: Some(f.sig.ident.span().start().line).filter(|&l| l > 0),
            }),
            java: None,
        },
        export_problem,
    })
}

/// Parsed contents of a `#[jni_mangle("pkg.Class"[, "name"][, "sig"])]` attribute.
struct JniMangle {
    /// Fully-qualified Java class, dotted (e.g. `example.Correct`).
    namespace: String,
    /// Explicit Java method name, if given (else derived from the fn name).
    method_name: Option<String>,
    /// Argument portion of an explicit JNI signature (e.g. `Ljava/lang/String;`),
    /// present only when the user disambiguates an overload. Drives the long
    /// `..._method__<args>` symbol form.
    arg_descriptor: Option<String>,
}

/// Find and parse a `#[jni_mangle(...)]` attribute, if present.
fn parse_jni_mangle(attrs: &[syn::Attribute]) -> Option<JniMangle> {
    let attr = attrs.iter().find(|a| {
        a.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "jni_mangle")
    })?;
    let args = attr
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::LitStr, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    let mut it = args.iter().map(|l| l.value());
    let namespace = it.next()?;
    // Per the jni docs: with two args, the 2nd is a signature iff it contains
    // '(', otherwise a method name; with three, they are name then signature.
    let (method_name, sig) = match (it.next(), it.next()) {
        (None, _) => (None, None),
        (Some(s), None) if s.contains('(') => (None, Some(s)),
        (Some(s), None) => (Some(s), None),
        (Some(name), Some(sig)) => (Some(name), Some(sig)),
    };
    Some(JniMangle {
        namespace,
        method_name,
        arg_descriptor: sig.and_then(|s| jni_sig_args(&s)),
    })
}

/// Extract the parameter list of a JNI method signature: `"(args)ret"` → `"args"`.
fn jni_sig_args(sig: &str) -> Option<String> {
    let start = sig.find('(')?;
    let end = sig.find(')')?;
    (start < end).then(|| sig[start + 1..end].to_string())
}

fn is_no_mangle(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        // Plain `#[no_mangle]`.
        if attr.path().is_ident("no_mangle") {
            return true;
        }
        // Edition 2024 `#[unsafe(no_mangle)]`: a list-style attribute whose path
        // is the `unsafe` keyword, wrapping the real attribute as its tokens.
        // Parse those tokens as a path so we match the `no_mangle` ident exactly
        // rather than substring-matching (which would trip on a doc comment that
        // merely mentions "no_mangle").
        if let syn::Meta::List(list) = &attr.meta
            && list.path.is_ident("unsafe")
        {
            return syn::parse2::<syn::Path>(list.tokens.clone())
                .is_ok_and(|p| p.is_ident("no_mangle"));
        }
        false
    })
}

fn is_system_abi(abi: &Option<syn::Abi>) -> bool {
    match abi {
        Some(abi) => match &abi.name {
            Some(name) => matches!(name.value().as_str(), "system" | "C"),
            None => true, // bare `extern` == "C"
        },
        None => false,
    }
}

fn receiver_kind(ty: &Type) -> Receiver {
    match last_segment_ident(ty).as_deref() {
        Some("JClass" | "jclass") => Receiver::Class,
        Some("JObject" | "jobject") => Receiver::Object,
        _ => Receiver::Unknown,
    }
}

/// Lower a `syn::Type` to our IR.
fn lower_type(ty: &Type) -> IrType {
    let Some((ident, args)) = path_ident_and_args(ty) else {
        return IrType::Unsupported(render(ty));
    };

    match ident.as_str() {
        "JRef" | "JMut" | "JOwned" => {
            let kind = match ident.as_str() {
                "JRef" => PointerKind::Ref,
                "JMut" => PointerKind::Mut,
                _ => PointerKind::Owned,
            };
            match first_type_arg(&args) {
                Some(inner) => IrType::Pointer(Pointer {
                    kind,
                    rust_type: typemap::normalize_rust_type(&render(inner)),
                    nullable: false,
                }),
                None => IrType::Unsupported(render(ty)),
            }
        }
        "Option" => match first_type_arg(&args) {
            Some(inner) => match lower_type(inner) {
                IrType::Pointer(mut p) => {
                    p.nullable = true;
                    IrType::Pointer(p)
                }
                other => other, // Option<JString> etc. — keep the object type
            },
            None => IrType::Unsupported(render(ty)),
        },
        other => {
            typemap::rust_simple_type(other).unwrap_or_else(|| IrType::Unsupported(render(ty)))
        }
    }
}

/// Final path segment ident + its angle-bracketed generic arguments.
fn path_ident_and_args(ty: &Type) -> Option<(String, Vec<GenericArgument>)> {
    let Type::Path(tp) = ty else { return None };
    let seg = tp.path.segments.last()?;
    let args = match &seg.arguments {
        PathArguments::AngleBracketed(a) => a.args.iter().cloned().collect(),
        _ => Vec::new(),
    };
    Some((seg.ident.to_string(), args))
}

fn last_segment_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(tp) => tp.path.segments.last().map(|s| s.ident.to_string()),
        Type::Reference(r) => last_segment_ident(&r.elem),
        _ => None,
    }
}

/// First *type* generic argument, skipping lifetimes/const generics.
fn first_type_arg(args: &[GenericArgument]) -> Option<&Type> {
    args.iter().find_map(|a| match a {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}

fn render(ty: &Type) -> String {
    ty.to_token_stream().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs_of(src: &str) -> Vec<syn::Attribute> {
        syn::parse_str::<syn::ItemFn>(src).unwrap().attrs
    }

    #[test]
    fn detects_both_no_mangle_spellings() {
        assert!(is_no_mangle(&attrs_of("#[no_mangle] fn f() {}")));
        assert!(is_no_mangle(&attrs_of("#[unsafe(no_mangle)] fn f() {}")));
    }

    #[test]
    fn ignores_attributes_that_merely_mention_no_mangle() {
        // A doc comment is `#[doc = "..."]`; substring matching would wrongly
        // treat this as exported.
        assert!(!is_no_mangle(&attrs_of(
            "#[doc = \"don't use no_mangle here\"] fn f() {}"
        )));
        assert!(!is_no_mangle(&attrs_of(
            "#[unsafe(export_name = \"x\")] fn f() {}"
        )));
        assert!(!is_no_mangle(&attrs_of("fn f() {}")));
    }

    fn lower_src(src: &str) -> Signature {
        let f: syn::ItemFn = syn::parse_str(src).unwrap();
        lower_fn(&f, std::path::Path::new("lib.rs")).expect("fn lowered")
    }

    #[test]
    fn jni_mangle_fn_lowers_to_mangled_symbol() {
        let sig = lower_src(
            r#"#[jni_mangle("example.MangleExample")]
               pub fn create<'local>(
                   mut env: EnvUnowned<'local>,
                   _class: JClass<'local>,
                   value: JString<'local>,
               ) -> JOwned<Box<String>> { unimplemented!() }"#,
        );
        assert_eq!(sig.key.symbol, "Java_example_MangleExample_create");
        assert_eq!(sig.key.java_class, "example/MangleExample");
        assert_eq!(sig.key.java_method, "create");
        assert_eq!(sig.receiver, Receiver::Class);
        assert_eq!(
            sig.params,
            vec![IrType::JavaObject {
                class: "java/lang/String".into()
            }]
        );
        assert!(sig.export_problem.is_none());
        match &sig.ret {
            IrType::Pointer(p) => {
                assert_eq!(p.kind, PointerKind::Owned);
                assert_eq!(p.rust_type, "Box<String>");
            }
            other => panic!("expected owned pointer, got {other:?}"),
        }
    }

    #[test]
    fn jni_mangle_derives_camel_case_method_name_and_skips_env_receiver() {
        let sig = lower_src(
            r#"#[jni_mangle("example.MangleExample")]
               pub fn set_value<'local>(
                   mut env: EnvUnowned<'local>,
                   _class: JClass<'local>,
                   ptr: JMut<'local, Box<String>>,
                   value: JString<'local>,
               ) { unimplemented!() }"#,
        );
        assert_eq!(sig.key.java_method, "setValue");
        assert_eq!(sig.key.symbol, "Java_example_MangleExample_setValue");
        assert_eq!(sig.params.len(), 2, "env + receiver skipped");
    }

    #[test]
    fn resolve_overloads_remangles_same_name_to_long_form() {
        // Two natives with the same Java (class, method) but different params:
        // each must get its long-form symbol, exactly as `java_loader` mangles a
        // class with two same-named natives.
        let mut sigs = vec![
            lower_src(
                r#"#[jni_mangle("example.Over", "combine")]
                   pub fn combine_ii<'l>(e: EnvUnowned<'l>, c: JClass<'l>, a: jint, b: jint) -> jint { unimplemented!() }"#,
            ),
            lower_src(
                r#"#[jni_mangle("example.Over", "combine")]
                   pub fn combine_ss<'l>(e: EnvUnowned<'l>, c: JClass<'l>, a: JString<'l>, b: JString<'l>) -> jint { unimplemented!() }"#,
            ),
        ];
        // Before resolution both mangle to the same short symbol (a collision).
        assert_eq!(sigs[0].key.symbol, sigs[1].key.symbol);

        resolve_overloads(&mut sigs);

        assert_eq!(sigs[0].key.symbol, "Java_example_Over_combine__II");
        assert_eq!(
            sigs[1].key.symbol,
            "Java_example_Over_combine__Ljava_lang_String_2Ljava_lang_String_2"
        );
        // The Rust origin's symbol is kept in sync for diagnostics.
        assert_eq!(
            sigs[0].origin.rust.as_ref().unwrap().symbol,
            sigs[0].key.symbol
        );
    }

    #[test]
    fn resolve_overloads_leaves_unique_names_short() {
        let mut sigs = vec![
            lower_src(
                r#"#[jni_mangle("example.Over")]
                   pub fn create<'l>(e: EnvUnowned<'l>, c: JClass<'l>, a: jint) -> jint { unimplemented!() }"#,
            ),
            lower_src(
                r#"#[jni_mangle("example.Over")]
                   pub fn destroy<'l>(e: EnvUnowned<'l>, c: JClass<'l>) { unimplemented!() }"#,
            ),
        ];
        resolve_overloads(&mut sigs);
        assert_eq!(sigs[0].key.symbol, "Java_example_Over_create");
        assert_eq!(sigs[1].key.symbol, "Java_example_Over_destroy");
    }

    #[test]
    fn jni_mangle_explicit_name_and_overload_signature() {
        // Explicit method name + JNI signature → long `__<args>` symbol form.
        let sig = lower_src(
            r#"#[jni_mangle("example.MangleExample", "lookup", "(Ljava/lang/String;)V")]
               pub fn lookup_impl<'local>(
                   mut env: EnvUnowned<'local>,
                   _class: JClass<'local>,
                   key: JString<'local>,
               ) { unimplemented!() }"#,
        );
        assert_eq!(
            sig.key.symbol,
            "Java_example_MangleExample_lookup__Ljava_lang_String_2"
        );
        assert_eq!(sig.key.java_method, "lookup");
    }
}
