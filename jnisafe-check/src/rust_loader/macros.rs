//! Recognize the jni 0.22.4 ergonomic macros as native-method declarations.
//!
//! `native_method! { … }` and the `native_methods { … }` block of
//! `bind_java_type! { … }` declare native methods with a small DSL instead of a
//! hand-written `Java_*` export. We parse those invocations and lower each into
//! the same [`Signature`] the legacy front-end produces, so the matcher in
//! `check.rs` is unchanged.
//!
//! Unlike a hand-written export, the DSL signature lists **only** the real Java
//! parameters (no env, no receiver), and names the class + method directly — so
//! we compute the join symbol with [`crate::mangle`] exactly as `java_loader`
//! does (short form; overloaded methods are a known gap, see the e2e tests).

use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::visit::Visit;
use syn::{Ident, Token, Type};

use crate::ir::{
    IrType, JavaRef, JavaRefKind, MethodKey, Origin, Receiver, RustExportProblem, Signature, SrcLoc,
};

mod kw {
    syn::custom_keyword!(raw);
    syn::custom_keyword!(non_null);
    syn::custom_keyword!(java_type);
    syn::custom_keyword!(name);
    syn::custom_keyword!(sig);
    syn::custom_keyword!(native_methods);
}

/// Walk a parsed file and collect both native methods (`native_method! { }` and
/// the `native_methods { }` block of `bind_java_type! { }`) as [`Signature`]s,
/// and the Rust→Java call bindings (`bind_java_type!`'s `methods`/`fields`/
/// `constructors` blocks) as [`JavaRef`]s.
pub fn collect(
    file: &syn::File,
    path: &Path,
    natives: &mut Vec<Signature>,
    refs: &mut Vec<JavaRef>,
) {
    MacroVisitor {
        path,
        natives,
        refs,
    }
    .visit_file(file);
}

struct MacroVisitor<'a> {
    path: &'a Path,
    natives: &'a mut Vec<Signature>,
    refs: &'a mut Vec<JavaRef>,
}

impl<'ast> Visit<'ast> for MacroVisitor<'_> {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let Some(name) = mac.path.segments.last().map(|s| s.ident.to_string()) else {
            return;
        };
        let line = mac
            .path
            .segments
            .last()
            .map(|s| s.ident.span().start().line)
            .filter(|&l| l > 0);
        match name.as_str() {
            "native_method" => {
                if let Some(sig) = parse_native_method(mac.tokens.clone(), self.path, line) {
                    self.natives.push(sig);
                }
            }
            "bind_java_type" => {
                parse_bind_java_type(mac.tokens.clone(), self.path, line, self.natives, self.refs);
            }
            _ => {}
        }
    }
}

/// One parsed native method: enough to build a [`Signature`]. `java_type` is the
/// dotted class; absent for `bind_java_type` entries (supplied by the header).
struct MethodSpec {
    java_type: Option<String>,
    /// Explicit `name = "…"`, else the raw fn name (camel-cased later).
    name_override: Option<String>,
    raw_fn_name: Option<String>,
    is_static: bool,
    params: Vec<IrType>,
    ret: IrType,
    /// True once we have a parameter/return signature (shorthand or `sig =`).
    have_sig: bool,
}

impl MethodSpec {
    fn new() -> Self {
        MethodSpec {
            java_type: None,
            name_override: None,
            raw_fn_name: None,
            is_static: false,
            params: Vec::new(),
            ret: IrType::Void,
            have_sig: false,
        }
    }

    fn method_name(&self) -> Option<String> {
        self.name_override.clone().or_else(|| {
            self.raw_fn_name
                .as_deref()
                .map(crate::mangle::snake_to_lower_camel)
        })
    }
}

/// Parse a whole `native_method! { … }` body. Properties and the inline `fn`
/// shorthand are independent top-level (comma-separated) entries that we merge.
fn parse_native_method(tokens: TokenStream, path: &Path, line: Option<usize>) -> Option<Signature> {
    let mut spec = MethodSpec::new();
    for entry in split_top_level_commas(tokens) {
        apply_entry(entry, &mut spec);
    }
    build_signature(&spec, spec.java_type.clone(), path, line)
}

/// Parse a `bind_java_type! { … }` body: pull the class from the header, then
/// emit a [`Signature`] for each `native_methods { … }` entry (Java→Rust) and a
/// [`JavaRef`] for each `methods`/`fields`/`constructors` entry (Rust→Java).
/// Other clauses (`type_map`, `is_instance_of`, `hooks`, config) are skipped.
fn parse_bind_java_type(
    tokens: TokenStream,
    path: &Path,
    line: Option<usize>,
    natives: &mut Vec<Signature>,
    refs: &mut Vec<JavaRef>,
) {
    let mut entries = split_top_level_commas(tokens).into_iter();
    let Some(class) = entries.next().and_then(bind_header_class) else {
        return;
    };
    let class_internal = crate::mangle::class_dotted_to_internal(&class);
    for entry in entries {
        let Some((keyword, block)) = block_keyword(&entry) else {
            continue;
        };
        match keyword.as_str() {
            "native_methods" => {
                for method in split_top_level_commas(block) {
                    let mut spec = MethodSpec::new();
                    apply_entry(method, &mut spec);
                    if let Some(sig) = build_signature(&spec, Some(class.clone()), path, line) {
                        natives.push(sig);
                    }
                }
            }
            "methods" => {
                for e in split_top_level_commas(block) {
                    if let Some(r) = parse_method_ref(e, &class_internal, path, line) {
                        refs.push(r);
                    }
                }
            }
            "constructors" => {
                for e in split_top_level_commas(block) {
                    if let Some(r) = parse_ctor_ref(e, &class_internal, path, line) {
                        refs.push(r);
                    }
                }
            }
            "fields" => {
                for e in split_top_level_commas(block) {
                    if let Some(r) = parse_field_ref(e, &class_internal, path, line) {
                        refs.push(r);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Build the [`SrcLoc`] origin for a Rust→Java reference (for diagnostics).
fn ref_loc(path: &Path, class_internal: &str, member: &str, line: Option<usize>) -> SrcLoc {
    SrcLoc {
        file: path.to_path_buf(),
        symbol: format!("{}.{member}", class_internal.replace('/', ".")),
        line,
    }
}

/// Lower a `methods { … }` entry into a [`JavaRef`] (a Java method Rust calls).
fn parse_method_ref(
    entry: TokenStream,
    class_internal: &str,
    path: &Path,
    line: Option<usize>,
) -> Option<JavaRef> {
    let Ok(Entry::Method {
        is_static,
        raw_fn_name,
        name_override,
        params,
        ret,
    }) = syn::parse2::<Entry>(entry)
    else {
        return None;
    };
    let java_name =
        name_override.unwrap_or_else(|| crate::mangle::snake_to_lower_camel(&raw_fn_name));
    Some(JavaRef {
        origin: ref_loc(path, class_internal, &java_name, line),
        class_internal: class_internal.to_owned(),
        kind: JavaRefKind::Method,
        java_name,
        is_static,
        params,
        ret,
        field_ty: None,
    })
}

/// Lower a `constructors { … }` entry into a [`JavaRef`]. The Java name is always
/// `<init>`; constructors are never static and their return is void.
fn parse_ctor_ref(
    entry: TokenStream,
    class_internal: &str,
    path: &Path,
    line: Option<usize>,
) -> Option<JavaRef> {
    let Ok(Entry::Method { params, .. }) = syn::parse2::<Entry>(entry) else {
        return None;
    };
    Some(JavaRef {
        origin: ref_loc(path, class_internal, "<init>", line),
        class_internal: class_internal.to_owned(),
        kind: JavaRefKind::Constructor,
        java_name: "<init>".to_owned(),
        is_static: false,
        params,
        ret: IrType::Void,
        field_ty: None,
    })
}

/// Lower a `fields { … }` entry into a [`JavaRef`] (a Java field Rust accesses).
fn parse_field_ref(
    entry: TokenStream,
    class_internal: &str,
    path: &Path,
    line: Option<usize>,
) -> Option<JavaRef> {
    let fe = syn::parse2::<FieldEntry>(entry).ok()?;
    let java_name = fe
        .name_override
        .unwrap_or_else(|| crate::mangle::snake_to_lower_camel(&fe.raw_name));
    Some(JavaRef {
        origin: ref_loc(path, class_internal, &java_name, line),
        class_internal: class_internal.to_owned(),
        kind: JavaRefKind::Field,
        java_name,
        is_static: fe.is_static,
        params: Vec::new(),
        ret: IrType::Void,
        field_ty: Some(fe.ty),
    })
}

/// Turn a fully-resolved [`MethodSpec`] into a [`Signature`]. Requires a class
/// and a method signature; otherwise the method can't be matched and is dropped.
fn build_signature(
    spec: &MethodSpec,
    java_type: Option<String>,
    path: &Path,
    line: Option<usize>,
) -> Option<Signature> {
    let java_type = java_type?;
    let method = spec.method_name()?;
    if !spec.have_sig {
        return None;
    }
    let class_internal = crate::mangle::class_dotted_to_internal(&java_type);
    let symbol = crate::mangle::mangle(&class_internal, &method, false, "");
    let receiver = if spec.is_static {
        Receiver::Class
    } else {
        Receiver::Object
    };
    Some(Signature {
        key: MethodKey {
            symbol: symbol.clone(),
            java_class: class_internal,
            java_method: method,
        },
        is_static: false,
        receiver,
        params: spec.params.clone(),
        ret: spec.ret.clone(),
        origin: Origin {
            rust: Some(SrcLoc {
                file: path.to_path_buf(),
                symbol,
                line,
            }),
            java: None,
        },
        export_problem: None::<RustExportProblem>,
    })
}

/// Classify and fold one top-level DSL entry into `spec`.
fn apply_entry(entry: TokenStream, spec: &mut MethodSpec) {
    match syn::parse2::<Entry>(entry) {
        Ok(Entry::JavaType(s)) => spec.java_type = Some(s),
        Ok(Entry::Name(s)) => spec.name_override = Some(s),
        Ok(Entry::StaticFlag(b)) => spec.is_static = b,
        Ok(Entry::Sig { params, ret }) => {
            spec.params = params;
            spec.ret = ret;
            spec.have_sig = true;
        }
        Ok(Entry::Method {
            is_static,
            raw_fn_name,
            name_override,
            params,
            ret,
        }) => {
            spec.is_static = spec.is_static || is_static;
            spec.raw_fn_name = Some(raw_fn_name);
            if let Some(n) = name_override {
                spec.name_override = Some(n);
            }
            spec.params = params;
            spec.ret = ret;
            spec.have_sig = true;
        }
        Ok(Entry::Other) | Err(_) => {}
    }
}

/// A single top-level entry of a `native_method!` / `native_methods` block.
enum Entry {
    JavaType(String),
    Name(String),
    StaticFlag(bool),
    Sig {
        params: Vec<IrType>,
        ret: IrType,
    },
    Method {
        is_static: bool,
        raw_fn_name: String,
        name_override: Option<String>,
        params: Vec<IrType>,
        ret: IrType,
    },
    Other,
}

impl Parse for Entry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Entry::Other);
        }
        // Property form: `ident = …` or `ident { … }`.
        let fork = input.fork();
        if fork.call(Ident::parse_any).is_ok() && fork.peek(Token![=]) {
            let kw = input.call(Ident::parse_any)?;
            input.parse::<Token![=]>()?;
            return Ok(match kw.to_string().as_str() {
                "java_type" => Entry::JavaType(parse_java_class_name(input)?),
                "name" => Entry::Name(input.parse::<syn::LitStr>()?.value()),
                "static" => Entry::StaticFlag(input.parse::<syn::LitBool>()?.value()),
                "sig" => {
                    let (params, ret) = parse_sig_body(input)?;
                    Entry::Sig { params, ret }
                }
                // type_map / error_policy / abi_check / catch_unwind / fn / rust_type / jni / …
                _ => Entry::Other,
            });
        }
        // Otherwise an inline method: `[static] [raw] [extern] fn …`.
        parse_method(input)
    }
}

/// Parse an inline method entry: `[static] [raw] [extern] fn [T::]name(args) -> ret`
/// or the block form `… fn [T::]name { sig = (args) -> ret, name = "…", … }`.
fn parse_method(input: ParseStream) -> syn::Result<Entry> {
    // Optional visibility on the generated wrapper (e.g. `pub fn …`).
    let _vis: syn::Visibility = input.parse()?;
    let mut is_static = false;
    loop {
        if input.peek(Token![static]) {
            input.parse::<Token![static]>()?;
            is_static = true;
        } else if input.peek(Token![extern]) {
            input.parse::<Token![extern]>()?;
        } else if input.peek(kw::raw) {
            input.parse::<kw::raw>()?;
        } else if input.peek(kw::non_null) {
            input.parse::<kw::non_null>()?;
        } else {
            break;
        }
    }
    if !input.peek(Token![fn]) {
        return Ok(Entry::Other);
    }
    input.parse::<Token![fn]>()?;

    let full_path: syn::Path = input.parse()?;
    let raw_fn_name = full_path.segments.last().unwrap().ident.to_string();

    if input.peek(syn::token::Paren) {
        // Shorthand: (params) [-> ret]
        let content;
        syn::parenthesized!(content in input);
        let params = parse_params(&content)?;
        let ret = parse_arrow_ret(input)?;
        Ok(Entry::Method {
            is_static,
            raw_fn_name,
            name_override: None,
            params,
            ret,
        })
    } else if input.peek(syn::token::Brace) {
        // Block: { sig = (params) -> ret, name = "…", static = …, fn = …, … }
        let body;
        syn::braced!(body in input);
        let mut block = MethodSpec::new();
        for e in split_top_level_commas(body.parse()?) {
            apply_entry(e, &mut block);
        }
        if !block.have_sig {
            return Ok(Entry::Other);
        }
        Ok(Entry::Method {
            is_static: is_static || block.is_static,
            raw_fn_name,
            name_override: block.name_override,
            params: block.params,
            ret: block.ret,
        })
    } else {
        Ok(Entry::Other)
    }
}

/// Parse a `(params) [-> ret]` signature body (the `sig = …` property form).
fn parse_sig_body(input: ParseStream) -> syn::Result<(Vec<IrType>, IrType)> {
    let content;
    syn::parenthesized!(content in input);
    let params = parse_params(&content)?;
    let ret = parse_arrow_ret(input)?;
    Ok((params, ret))
}

/// Parse an optional `-> Type` return (absent ⇒ void).
fn parse_arrow_ret(input: ParseStream) -> syn::Result<IrType> {
    if input.peek(Token![->]) {
        input.parse::<Token![->]>()?;
        let ty: Type = input.parse()?;
        Ok(lower_ret(&ty))
    } else {
        Ok(IrType::Void)
    }
}

/// Parse a comma-separated parameter list of `[name :] [&] TYPE`, lowering each
/// type with the shared [`super::lower_type`].
fn parse_params(content: ParseStream) -> syn::Result<Vec<IrType>> {
    let mut out = Vec::new();
    while !content.is_empty() {
        // Optional `name :` (a single colon distinguishes it from a `::` path).
        if content.peek(Ident) && content.peek2(Token![:]) {
            content.call(Ident::parse_any)?;
            content.parse::<Token![:]>()?;
        }
        // Optional leading `&` is ignored per the jni DSL.
        if content.peek(Token![&]) {
            content.parse::<Token![&]>()?;
        }
        let ty: Type = content.parse()?;
        out.push(super::lower_type(&ty));
        if !content.is_empty() {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(out)
}

/// Lower a return type, mapping `void` and `()` to [`IrType::Void`].
fn lower_ret(ty: &Type) -> IrType {
    match ty {
        Type::Tuple(t) if t.elems.is_empty() => IrType::Void,
        Type::Path(p) if p.path.is_ident("void") => IrType::Void,
        _ => super::lower_type(ty),
    }
}

/// Parse a Java class name: a string literal (`"a.b.C"`) or dotted idents (`a.b.C`).
fn parse_java_class_name(input: ParseStream) -> syn::Result<String> {
    if input.peek(syn::LitStr) {
        return Ok(input.parse::<syn::LitStr>()?.value());
    }
    let mut parts = vec![input.call(Ident::parse_any)?.to_string()];
    while input.peek(Token![.]) {
        input.parse::<Token![.]>()?;
        parts.push(input.call(Ident::parse_any)?.to_string());
    }
    Ok(parts.join("."))
}

/// Extract the class from a `bind_java_type!` header entry, in either the
/// shorthand `[pub] RustType => <class>` form or the `java_type = <class>` form.
fn bind_header_class(entry: TokenStream) -> Option<String> {
    syn::parse2::<BindHeader>(entry).ok().map(|h| h.class)
}

struct BindHeader {
    class: String,
}

impl Parse for BindHeader {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Optional `#[…]` attributes on the macro body.
        while input.peek(Token![#]) {
            input.parse::<Token![#]>()?;
            let _bracket;
            syn::bracketed!(_bracket in input);
            let _: TokenStream = _bracket.parse()?;
        }
        // `java_type = <class>` form.
        if input.peek(kw::java_type) {
            input.parse::<kw::java_type>()?;
            input.parse::<Token![=]>()?;
            return Ok(BindHeader {
                class: parse_java_class_name(input)?,
            });
        }
        // Shorthand `[pub] RustType => <class>` form.
        let _vis: syn::Visibility = input.parse()?;
        let _rust_type: syn::Path = input.parse()?;
        input.parse::<Token![=>]>()?;
        Ok(BindHeader {
            class: parse_java_class_name(input)?,
        })
    }
}

/// If `entry` is `keyword { … }` or `keyword = { … }`, return `(keyword, inner)`.
/// (`bind_java_type!` accepts both spellings for its block clauses.)
fn block_keyword(entry: &TokenStream) -> Option<(String, TokenStream)> {
    let mut it = entry.clone().into_iter();
    let keyword = match it.next()? {
        TokenTree::Ident(id) => id.to_string(),
        _ => return None,
    };
    let mut next = it.next()?;
    if let TokenTree::Punct(p) = &next
        && p.as_char() == '='
    {
        next = it.next()?;
    }
    match next {
        TokenTree::Group(g) if g.delimiter() == proc_macro2::Delimiter::Brace => {
            Some((keyword, g.stream()))
        }
        _ => None,
    }
}

/// One `fields { … }` entry: `[vis] [static] [non_null] name: Type` or the block
/// form `name { sig = Type, name = "…", get = …, set = …, static = … }`.
struct FieldEntry {
    is_static: bool,
    /// The Rust field name (camel-cased into the Java name unless overridden).
    raw_name: String,
    name_override: Option<String>,
    ty: IrType,
}

impl Parse for FieldEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let _vis: syn::Visibility = input.parse()?;
        let mut is_static = false;
        loop {
            if input.peek(Token![static]) {
                input.parse::<Token![static]>()?;
                is_static = true;
            } else if input.peek(kw::non_null) {
                input.parse::<kw::non_null>()?;
            } else {
                break;
            }
        }
        let raw_name = input.call(Ident::parse_any)?.to_string();

        if input.peek(syn::token::Brace) {
            // Block form: only `sig`/`name`/`static` matter; `get`/`set`/etc. are
            // generated-accessor names that don't affect what Java field we look up.
            let body;
            syn::braced!(body in input);
            let mut ty = None;
            let mut name_override = None;
            let mut blk_static = false;
            for e in split_top_level_commas(body.parse()?) {
                match syn::parse2::<FieldProp>(e) {
                    Ok(FieldProp::Sig(t)) => ty = Some(t),
                    Ok(FieldProp::Name(n)) => name_override = Some(n),
                    Ok(FieldProp::Static(b)) => blk_static = b,
                    Ok(FieldProp::Other) | Err(_) => {}
                }
            }
            let ty = ty.ok_or_else(|| input.error("field block missing `sig = Type`"))?;
            return Ok(FieldEntry {
                is_static: is_static || blk_static,
                raw_name,
                name_override,
                ty,
            });
        }

        // Shorthand: `name: Type`.
        input.parse::<Token![:]>()?;
        let ty: Type = input.parse()?;
        Ok(FieldEntry {
            is_static,
            raw_name,
            name_override: None,
            ty: super::lower_type(&ty),
        })
    }
}

/// A property inside a `fields { name { … } }` block we care about.
enum FieldProp {
    Sig(IrType),
    Name(String),
    Static(bool),
    Other,
}

impl Parse for FieldProp {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(kw::sig) {
            input.parse::<kw::sig>()?;
            input.parse::<Token![=]>()?;
            let ty: Type = input.parse()?;
            return Ok(FieldProp::Sig(super::lower_type(&ty)));
        }
        if input.peek(kw::name) {
            input.parse::<kw::name>()?;
            input.parse::<Token![=]>()?;
            return Ok(FieldProp::Name(input.parse::<syn::LitStr>()?.value()));
        }
        if input.peek(Token![static]) {
            input.parse::<Token![static]>()?;
            input.parse::<Token![=]>()?;
            return Ok(FieldProp::Static(input.parse::<syn::LitBool>()?.value()));
        }
        Ok(FieldProp::Other)
    }
}

/// Split a token stream on top-level commas. Commas nested inside `{}`/`()`/`[]`
/// live in a [`TokenTree::Group`], so a simple scan over the top-level trees is
/// enough — no depth tracking needed.
fn split_top_level_commas(ts: TokenStream) -> Vec<TokenStream> {
    let mut entries = Vec::new();
    let mut cur = TokenStream::new();
    for tt in ts {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == ',' => {
                if !cur.is_empty() {
                    entries.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.extend(std::iter::once(tt)),
        }
    }
    if !cur.is_empty() {
        entries.push(cur);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Pointer, PointerKind};

    fn collect_src(src: &str) -> Vec<Signature> {
        collect_all(src).0
    }

    fn collect_all(src: &str) -> (Vec<Signature>, Vec<JavaRef>) {
        let file = syn::parse_file(src).unwrap();
        let mut natives = Vec::new();
        let mut refs = Vec::new();
        collect(&file, Path::new("lib.rs"), &mut natives, &mut refs);
        (natives, refs)
    }

    fn find<'a>(sigs: &'a [Signature], method: &str) -> &'a Signature {
        sigs.iter()
            .find(|s| s.key.java_method == method)
            .unwrap_or_else(|| panic!("no method {method}; got {sigs:?}"))
    }

    #[test]
    fn native_method_primitives() {
        let sigs = collect_src(
            r#"
            const M: jni::NativeMethod = native_method! {
                java_type = "example.NativeMethodExample",
                extern fn native_add(a: jint, b: jint) -> jint,
            };
            "#,
        );
        let add = find(&sigs, "nativeAdd");
        assert_eq!(add.key.symbol, "Java_example_NativeMethodExample_nativeAdd");
        assert_eq!(add.key.java_class, "example/NativeMethodExample");
        assert_eq!(add.receiver, Receiver::Object); // instance (no `static`)
        assert_eq!(
            add.params,
            vec![
                IrType::Primitive(crate::ir::Primitive::Int),
                IrType::Primitive(crate::ir::Primitive::Int)
            ]
        );
        assert_eq!(add.ret, IrType::Primitive(crate::ir::Primitive::Int));
    }

    #[test]
    fn native_method_recognizes_jnisafe_handle_via_type_map() {
        // The `type_map` entry is for the jni macro only; the checker reads the
        // jnisafe types written in the signature, which lower as usual.
        let sigs = collect_src(
            r#"
            const M: jni::NativeMethod = native_method! {
                java_type = "example.NativeMethodExample",
                type_map = { unsafe JOwned<Box<String>> => long },
                static extern fn create(value: JString) -> JOwned<Box<String>>,
            };
            "#,
        );
        let create = find(&sigs, "create");
        assert_eq!(create.receiver, Receiver::Class); // `static`
        assert_eq!(
            create.params,
            vec![IrType::JavaObject {
                class: "java/lang/String".into()
            }]
        );
        match &create.ret {
            IrType::Pointer(Pointer {
                kind, rust_type, ..
            }) => {
                assert_eq!(*kind, PointerKind::Owned);
                assert_eq!(rust_type, "Box<String>");
            }
            other => panic!("expected owned pointer, got {other:?}"),
        }
    }

    #[test]
    fn native_method_name_override_in_array() {
        let sigs = collect_src(
            r#"
            const METHODS: &[jni::NativeMethod] = &[
                native_method! { java_type = "example.X", name = "ping", fn pong() },
                native_method! { java_type = "example.X", static fn get_value() -> jlong },
            ];
            "#,
        );
        assert_eq!(find(&sigs, "ping").key.symbol, "Java_example_X_ping");
        assert_eq!(find(&sigs, "getValue").receiver, Receiver::Class);
    }

    #[test]
    fn bind_java_type_native_methods_block() {
        let sigs = collect_src(
            r#"
            bind_java_type! {
                pub Calculator => "example.BindTypeExample",
                constructors { fn new() },
                methods { fn unrelated() -> jint },
                native_methods {
                    extern fn native_add(a: jint, b: jint) -> jint,
                    static extern fn native_create(value: JString) -> JOwned<Box<String>>,
                },
            }
            "#,
        );
        // Only native_methods are lowered, not constructors/methods.
        assert_eq!(sigs.len(), 2, "got {sigs:?}");
        assert_eq!(
            find(&sigs, "nativeAdd").key.symbol,
            "Java_example_BindTypeExample_nativeAdd"
        );
        let create = find(&sigs, "nativeCreate");
        assert_eq!(create.receiver, Receiver::Class);
        assert!(matches!(create.ret, IrType::Pointer(_)));
    }

    #[test]
    fn bind_java_type_collects_rust_to_java_call_refs() {
        use crate::ir::Primitive::Int;

        let (natives, refs) = collect_all(
            r#"
            bind_java_type! {
                pub Foo => "example.Foo",
                type_map = { unsafe JOwned<Box<String>> => long },
                methods {
                    static fn doubled(x: jint) -> jint,
                    pub fn get_name() -> JString,
                },
                fields {
                    static counter: jint,
                    value { sig = jint, name = "theValue" },
                },
                constructors { fn new(value: jint) },
                native_methods {
                    static extern fn native_add(a: jint, b: jint) -> jint,
                },
            }
            "#,
        );

        // The native_methods entry is a Signature, not a ref.
        assert_eq!(natives.len(), 1, "got {natives:?}");
        // methods (2) + fields (2) + constructors (1).
        assert_eq!(refs.len(), 5, "got {refs:?}");

        let find = |kind: JavaRefKind, name: &str| {
            refs.iter()
                .find(|r| r.kind == kind && r.java_name == name)
                .unwrap_or_else(|| panic!("missing {kind:?} {name}; got {refs:?}"))
        };

        let doubled = find(JavaRefKind::Method, "doubled");
        assert!(doubled.is_static);
        assert_eq!(doubled.class_internal, "example/Foo");
        assert_eq!(doubled.params, vec![IrType::Primitive(Int)]);
        assert_eq!(doubled.ret, IrType::Primitive(Int));

        // snake_case fn name → lowerCamelCase Java name; object return.
        let get_name = find(JavaRefKind::Method, "getName");
        assert!(!get_name.is_static);
        assert_eq!(
            get_name.ret,
            IrType::JavaObject {
                class: "java/lang/String".into()
            }
        );

        let counter = find(JavaRefKind::Field, "counter");
        assert!(counter.is_static);
        assert_eq!(counter.field_ty, Some(IrType::Primitive(Int)));

        // Block form with an explicit Java name override.
        let the_value = find(JavaRefKind::Field, "theValue");
        assert!(!the_value.is_static);
        assert_eq!(the_value.field_ty, Some(IrType::Primitive(Int)));

        let ctor = find(JavaRefKind::Constructor, "<init>");
        assert!(!ctor.is_static);
        assert_eq!(ctor.params, vec![IrType::Primitive(Int)]);
        assert_eq!(ctor.ret, IrType::Void);
    }

    #[test]
    fn bind_java_type_java_type_property_and_block_method() {
        let sigs = collect_src(
            r#"
            bind_java_type! {
                java_type = example.BindTypeExample,
                native_methods {
                    fn native_square { sig = (value: jint) -> jint, fn = square_impl },
                },
            }
            "#,
        );
        let sq = find(&sigs, "nativeSquare");
        assert_eq!(sq.key.java_class, "example/BindTypeExample");
        assert_eq!(
            sq.params,
            vec![IrType::Primitive(crate::ir::Primitive::Int)]
        );
    }
}
