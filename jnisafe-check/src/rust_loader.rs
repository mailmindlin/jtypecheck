//! Rust front-end: walk a crate's source with `syn`, find exported JNI
//! functions (`#[no_mangle] extern "system" fn Java_*`), and lower each into a
//! [`Signature`]. Hidden behind the [`RustExtractor`] trait so a future
//! rust-analyzer backend can replace it without touching the matcher.

use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::{FnArg, GenericArgument, Item, PathArguments, ReturnType, Type};

use crate::ir::{IrType, MethodKey, Origin, Pointer, PointerKind, Receiver, Signature, SrcLoc};
use crate::typemap;

#[derive(Debug, thiserror::Error)]
pub enum RustLoadError {
    #[error("failed to read `{0}`: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("failed to parse `{0}`: {1}")]
    Parse(PathBuf, syn::Error),
}

/// Abstraction over "give me the exported JNI functions of a crate."
pub trait RustExtractor {
    fn extract(&self, crate_dir: &Path) -> Result<Vec<Signature>, RustLoadError>;
}

/// Default `syn`-based source extractor.
pub struct SynBackend;

impl RustExtractor for SynBackend {
    fn extract(&self, crate_dir: &Path) -> Result<Vec<Signature>, RustLoadError> {
        let mut sigs = Vec::new();
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
            extract_file(&root, &mut sigs)?;
        } else {
            for entry in walkdir::WalkDir::new(&root).into_iter().flatten() {
                if entry.path().extension().is_some_and(|e| e == "rs") {
                    extract_file(entry.path(), &mut sigs)?;
                }
            }
        }
        Ok(sigs)
    }
}

fn extract_file(path: &Path, out: &mut Vec<Signature>) -> Result<(), RustLoadError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| RustLoadError::Io(path.to_path_buf(), e))?;
    let file =
        syn::parse_file(&content).map_err(|e| RustLoadError::Parse(path.to_path_buf(), e))?;
    for item in &file.items {
        let Item::Fn(f) = item else { continue };
        if let Some(sig) = lower_fn(f, path) {
            out.push(sig);
        }
    }
    Ok(())
}

fn lower_fn(f: &syn::ItemFn, path: &Path) -> Option<Signature> {
    let ident = f.sig.ident.to_string();
    if !ident.starts_with("Java_") {
        return None;
    }
    if !is_no_mangle(&f.attrs) || !is_system_abi(&f.sig.abi) {
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

    Some(Signature {
        key: MethodKey {
            symbol: ident.clone(),
            java_class: String::new(),
            java_method: String::new(),
        },
        is_static: false,
        receiver,
        params,
        ret,
        origin: Origin {
            rust: Some(SrcLoc {
                file: path.to_path_buf(),
                symbol: ident,
                line: Some(f.sig.ident.span().start().line).filter(|&l| l > 0),
            }),
            java: None,
        },
    })
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
}
