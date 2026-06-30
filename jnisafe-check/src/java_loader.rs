//! Java front-end: read `.class` / `.jar` with cafebabe and lower native
//! methods (plus their `@Ref`/`@Mut`/`@Owned` type annotations) into [`Signature`]s.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use cafebabe::attributes::{
    AnnotationElementValue, AttributeData, TypeAnnotation, TypeAnnotationTarget,
};
use cafebabe::bytecode::Opcode;
use cafebabe::descriptors::{FieldDescriptor, FieldType, MethodDescriptor, ReturnDescriptor};
use cafebabe::{FieldAccessFlags, MethodAccessFlags, parse_class};

use crate::code::{
    self, ExceptionRange, Insn, LocalHandleAnn, LocalName, MemberRef, MethodCode, Op, ParamInfo,
};
use crate::ir::{
    IrType, JavaClassModel, JavaFieldSig, JavaLoc, JavaMethodSig, MethodKey, Origin, Pointer,
    PointerKind, Receiver, Signature,
};
use crate::mangle;
use crate::typemap;

/// Package the annotations live in (binary form).
const ANN_PKG: &str = "io/github/mailmindlin/jnisafe";

#[derive(Debug, thiserror::Error)]
pub enum JavaLoadError {
    #[error("failed to read `{0}`: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("failed to open jar `{0}`: {1}")]
    Jar(PathBuf, zip::result::ZipError),
    #[error("failed to parse class in `{0}`: {1}")]
    Parse(String, String),
}

/// Load every native method from the given paths (`.class` files, directories
/// of `.class` files, or `.jar` archives).
pub fn load(paths: &[PathBuf]) -> Result<Vec<Signature>, JavaLoadError> {
    let mut sigs = Vec::new();
    visit_classes(paths, |class| {
        lower_natives(class, &mut sigs);
        Ok(())
    })?;
    Ok(sigs)
}

/// Build a [`JavaClassModel`] — the callable surface (all methods, fields, and
/// `<init>` constructors) — for every class in the given paths. Used to verify
/// the Rust→Java bindings declared by `bind_java_type!`'s
/// `methods`/`fields`/`constructors` clauses.
pub fn load_models(paths: &[PathBuf]) -> Result<Vec<JavaClassModel>, JavaLoadError> {
    let mut models = Vec::new();
    visit_classes(paths, |class| {
        models.push(build_model(class));
        Ok(())
    })?;
    Ok(models)
}

fn read_file(path: &Path) -> Result<Vec<u8>, JavaLoadError> {
    std::fs::read(path).map_err(|e| JavaLoadError::Io(path.to_path_buf(), e))
}

/// Walk `.class` / directory / `.jar` inputs, parse each class, and hand it to
/// `f`. Shared by [`load`] and [`load_models`].
fn visit_classes<F>(paths: &[PathBuf], mut f: F) -> Result<(), JavaLoadError>
where
    F: FnMut(&cafebabe::ClassFile) -> Result<(), JavaLoadError>,
{
    for path in paths {
        if path.is_dir() {
            for entry in walkdir::WalkDir::new(path).into_iter().flatten() {
                if entry.path().extension().is_some_and(|e| e == "class") {
                    let bytes = read_file(entry.path())?;
                    let class = parse_one(&bytes, entry.path().display().to_string())?;
                    f(&class)?;
                }
            }
        } else if path.extension().is_some_and(|e| e == "jar") {
            let file =
                std::fs::File::open(path).map_err(|e| JavaLoadError::Io(path.to_path_buf(), e))?;
            let mut zip = zip::ZipArchive::new(file)
                .map_err(|e| JavaLoadError::Jar(path.to_path_buf(), e))?;
            for i in 0..zip.len() {
                let mut entry = zip
                    .by_index(i)
                    .map_err(|e| JavaLoadError::Jar(path.to_path_buf(), e))?;
                if !entry.name().ends_with(".class") {
                    continue;
                }
                let source = format!("{}!{}", path.display(), entry.name());
                let mut bytes = Vec::with_capacity(entry.size() as usize);
                entry
                    .read_to_end(&mut bytes)
                    .map_err(|e| JavaLoadError::Io(path.to_path_buf(), e))?;
                let class = parse_one(&bytes, source)?;
                f(&class)?;
            }
        } else {
            let bytes = read_file(path)?;
            let class = parse_one(&bytes, path.display().to_string())?;
            f(&class)?;
        }
    }
    Ok(())
}

/// Parse one class, attributing a parse error to `source`.
fn parse_one(bytes: &[u8], source: String) -> Result<cafebabe::ClassFile<'_>, JavaLoadError> {
    parse_class(bytes).map_err(|e| JavaLoadError::Parse(source, e.to_string()))
}

/// Build the callable surface of one class for the Rust→Java check.
fn build_model(class: &cafebabe::ClassFile) -> JavaClassModel {
    let mut methods = Vec::new();
    let mut constructors = Vec::new();
    for m in &class.methods {
        let name = m.name.as_ref();
        // Static initializers are never callable from Rust.
        if name == "<clinit>" {
            continue;
        }
        let arg = arg_descriptor(&m.descriptor);
        if name == "<init>" {
            constructors.push(arg);
        } else {
            // Include native *and* non-native methods — JNI can call either, and
            // it bypasses Java access control, so don't filter by visibility.
            methods.push(JavaMethodSig {
                name: name.to_owned(),
                is_static: m.access_flags.contains(MethodAccessFlags::STATIC),
                arg_descriptor: arg,
                ret_descriptor: ret_descriptor(&m.descriptor),
            });
        }
    }
    let fields = class
        .fields
        .iter()
        .map(|fd| JavaFieldSig {
            name: fd.name.to_string(),
            is_static: fd.access_flags.contains(FieldAccessFlags::STATIC),
            descriptor: fd.descriptor.to_string(),
            annotation: field_pointer_annotation(&fd.attributes),
        })
        .collect();
    JavaClassModel {
        internal_name: class.this_class.to_string(),
        methods,
        fields,
        constructors,
    }
}

/// Lower the native methods of one parsed class into [`Signature`]s.
fn lower_natives(class: &cafebabe::ClassFile, out: &mut Vec<Signature>) {
    let class_name = class.this_class.to_string();

    // First pass: count native methods by name so we know which need long-form
    // (overloaded) mangling.
    let mut name_counts: HashMap<&str, usize> = HashMap::new();
    for m in &class.methods {
        if m.access_flags.contains(MethodAccessFlags::NATIVE) {
            *name_counts.entry(m.name.as_ref()).or_insert(0) += 1;
        }
    }

    for m in &class.methods {
        if !m.access_flags.contains(MethodAccessFlags::NATIVE) {
            continue;
        }
        if has_ignore(&m.attributes) {
            continue;
        }

        let is_static = m.access_flags.contains(MethodAccessFlags::STATIC);
        let overloaded = name_counts.get(m.name.as_ref()).copied().unwrap_or(0) > 1;
        let arg_desc = arg_descriptor(&m.descriptor);
        let symbol = mangle::mangle(&class_name, &m.name, overloaded, &arg_desc);

        let anns = collect_pointer_annotations(&m.attributes);
        let params = lower_params(&m.descriptor, &anns);
        let ret = lower_return(&m.descriptor, &anns.ret);

        out.push(Signature {
            key: MethodKey {
                symbol,
                java_class: class_name.clone(),
                java_method: m.name.to_string(),
            },
            is_static,
            receiver: Receiver::Unknown,
            params,
            ret,
            origin: Origin {
                rust: None,
                java: Some(JavaLoc {
                    class: class_name.clone(),
                    method: m.name.to_string(),
                    descriptor: full_descriptor(&m.descriptor),
                }),
            },
            export_problem: None,
        });
    }
}

/// Returns true when the method carries `@io.github.mailmindlin.jnisafe.Ignore`.
fn has_ignore(attrs: &[cafebabe::attributes::AttributeInfo]) -> bool {
    for a in attrs {
        if let AttributeData::RuntimeInvisibleAnnotations(anns)
        | AttributeData::RuntimeVisibleAnnotations(anns) = &a.data
        {
            for ann in anns {
                if annotation_class(&ann.type_descriptor) == format!("{ANN_PKG}/Ignore") {
                    return true;
                }
            }
        }
    }
    false
}

/// Read the `@SuppressJni` categories declared on an element (a class, method, or
/// field), normalising the descriptive synonyms to their canonical key. Empty if
/// the element carries no `@SuppressJni`.
fn suppress_categories(attrs: &[cafebabe::attributes::AttributeInfo]) -> Vec<String> {
    let mut cats = Vec::new();
    for a in attrs {
        let anns = match &a.data {
            AttributeData::RuntimeInvisibleAnnotations(v)
            | AttributeData::RuntimeVisibleAnnotations(v) => v,
            _ => continue,
        };
        for ann in anns {
            if annotation_class(&ann.type_descriptor) != format!("{ANN_PKG}/SuppressJni") {
                continue;
            }
            for el in &ann.elements {
                if el.name.as_ref() != "value" {
                    continue;
                }
                if let AnnotationElementValue::ArrayValue(items) = &el.value {
                    for item in items {
                        if let AnnotationElementValue::StringConstant(s) = item {
                            cats.push(normalize_category(s));
                        }
                    }
                }
            }
        }
    }
    cats
}

/// Canonicalise a `@SuppressJni` category, mapping the descriptive synonyms
/// (`"type"`, `"leak"`) onto the Rust-flavoured keys the checker gates on.
fn normalize_category(raw: &str) -> String {
    match raw {
        "type" => "transmute",
        "leak" => "forget",
        other => other,
    }
    .to_owned()
}

/// A pointer annotation recovered from the bytecode type-annotation table.
struct PtrAnn {
    kind: PointerKind,
    rust_type: String,
    nullable: bool,
}

#[derive(Default)]
struct MethodAnns {
    params: HashMap<usize, PtrAnn>,
    ret: Option<PtrAnn>,
}

fn collect_pointer_annotations(attrs: &[cafebabe::attributes::AttributeInfo]) -> MethodAnns {
    let mut result = MethodAnns::default();
    for a in attrs {
        let tas: &[TypeAnnotation] = match &a.data {
            AttributeData::RuntimeInvisibleTypeAnnotations(v)
            | AttributeData::RuntimeVisibleTypeAnnotations(v) => v,
            _ => continue,
        };
        for ta in tas {
            // Only top-level annotations on the type itself (not nested type args).
            if !ta.target_path.is_empty() {
                continue;
            }
            let Some(kind) = pointer_kind(&annotation_class(&ta.annotation.type_descriptor)) else {
                continue;
            };
            let (rust_type, nullable) = read_members(&ta.annotation);
            let ann = PtrAnn {
                kind,
                rust_type,
                nullable,
            };
            match ta.target_type {
                TypeAnnotationTarget::FormalParameter { index } => {
                    result.params.insert(index as usize, ann);
                }
                TypeAnnotationTarget::Empty => {
                    // Method return type (target_type 0x14 folds into Empty).
                    result.ret = Some(ann);
                }
                _ => {}
            }
        }
    }
    result
}

/// Recover a top-level `@Ref`/`@Mut`/`@Owned` annotation on a *field*'s type, if
/// any. A field-declaration type annotation uses target_type `0x13`, which
/// cafebabe (like a method return, `0x14`) represents as the empty target — so
/// we accept [`TypeAnnotationTarget::Empty`]. Annotation kind/type validation
/// (e.g. it must sit on a `long`) is left to the cross-check; here we only
/// surface what the field declares.
fn field_pointer_annotation(attrs: &[cafebabe::attributes::AttributeInfo]) -> Option<Pointer> {
    for a in attrs {
        let tas: &[TypeAnnotation] = match &a.data {
            AttributeData::RuntimeInvisibleTypeAnnotations(v)
            | AttributeData::RuntimeVisibleTypeAnnotations(v) => v,
            _ => continue,
        };
        for ta in tas {
            if !ta.target_path.is_empty() || !matches!(ta.target_type, TypeAnnotationTarget::Empty)
            {
                continue;
            }
            let Some(kind) = pointer_kind(&annotation_class(&ta.annotation.type_descriptor)) else {
                continue;
            };
            let (rust_type, nullable) = read_members(&ta.annotation);
            return Some(Pointer {
                kind,
                rust_type,
                nullable,
            });
        }
    }
    None
}

/// Extract `value` (the Rust type, normalized) and `nullable` (default true).
fn read_members(ann: &cafebabe::attributes::Annotation) -> (String, bool) {
    let mut rust_type = String::new();
    let mut nullable = true;
    for el in &ann.elements {
        match (el.name.as_ref(), &el.value) {
            ("value", AnnotationElementValue::StringConstant(s)) => {
                rust_type = typemap::normalize_rust_type(s);
            }
            ("nullable", AnnotationElementValue::BooleanConstant(b)) => {
                nullable = *b != 0;
            }
            _ => {}
        }
    }
    (rust_type, nullable)
}

fn lower_params(desc: &MethodDescriptor, anns: &MethodAnns) -> Vec<IrType> {
    desc.parameters
        .iter()
        .enumerate()
        .map(|(i, fd)| match anns.params.get(&i) {
            Some(ann) => pointer_ir(fd, ann),
            None => java_field_to_ir(fd),
        })
        .collect()
}

fn lower_return(desc: &MethodDescriptor, ret_ann: &Option<PtrAnn>) -> IrType {
    match &desc.return_type {
        ReturnDescriptor::Void => IrType::Void,
        ReturnDescriptor::Return(fd) => match ret_ann {
            Some(ann) => pointer_ir(fd, ann),
            None => java_field_to_ir(fd),
        },
    }
}

/// Build a `Pointer` IR, validating that the annotated slot is a bare `long`.
/// An annotation on anything else becomes `Misannotated` (E026).
fn pointer_ir(fd: &FieldDescriptor, ann: &PtrAnn) -> IrType {
    let is_long = fd.dimensions == 0 && matches!(fd.field_type, FieldType::Long);
    if !is_long {
        let narrow_int = fd.dimensions == 0
            && matches!(
                fd.field_type,
                FieldType::Integer | FieldType::Short | FieldType::Byte | FieldType::Char
            );
        return IrType::Misannotated {
            ann_kind: ann.kind,
            java_desc: java_type_name(fd),
            narrow_int,
        };
    }
    IrType::Pointer(Pointer {
        kind: ann.kind,
        rust_type: ann.rust_type.clone(),
        nullable: ann.nullable,
    })
}

/// Human-readable Java type for a field descriptor, e.g. `int`,
/// `java.lang.String`, `long[]` (for diagnostics — not the raw JVM descriptor).
fn java_type_name(fd: &FieldDescriptor) -> String {
    let base = match &fd.field_type {
        FieldType::Boolean => "boolean".to_owned(),
        FieldType::Byte => "byte".to_owned(),
        FieldType::Char => "char".to_owned(),
        FieldType::Short => "short".to_owned(),
        FieldType::Integer => "int".to_owned(),
        FieldType::Long => "long".to_owned(),
        FieldType::Float => "float".to_owned(),
        FieldType::Double => "double".to_owned(),
        FieldType::Object(cn) => cn.to_string().replace('/', "."),
    };
    format!("{base}{}", "[]".repeat(fd.dimensions as usize))
}

fn java_field_to_ir(fd: &FieldDescriptor) -> IrType {
    if fd.dimensions > 0 {
        // Arrays keyed by their full JVM descriptor, e.g. "[Ljava/lang/String;".
        return IrType::JavaObject {
            class: fd.to_string(),
        };
    }
    match &fd.field_type {
        FieldType::Object(cn) => IrType::JavaObject {
            class: cn.to_string(),
        },
        other => {
            let c = other.to_string().chars().next().unwrap_or('?');
            match typemap::primitive_from_descriptor_char(c) {
                Some(p) => IrType::Primitive(p),
                None => IrType::Unsupported(fd.to_string()),
            }
        }
    }
}

/// `io/github/mailmindlin/jnisafe/Ref` from an annotation's `FieldDescriptor`.
fn annotation_class(fd: &FieldDescriptor) -> String {
    match &fd.field_type {
        FieldType::Object(cn) => cn.to_string(),
        _ => String::new(),
    }
}

fn pointer_kind(class: &str) -> Option<PointerKind> {
    match class.strip_prefix(&format!("{ANN_PKG}/"))? {
        "Ref" => Some(PointerKind::Ref),
        "Mut" => Some(PointerKind::Mut),
        "Owned" => Some(PointerKind::Owned),
        _ => None,
    }
}

/// Concatenated parameter descriptors (no parens), for overload mangling.
fn arg_descriptor(desc: &MethodDescriptor) -> String {
    desc.parameters.iter().map(|fd| fd.to_string()).collect()
}

/// The return-type descriptor, e.g. `"I"` or `"V"`.
fn ret_descriptor(desc: &MethodDescriptor) -> String {
    match &desc.return_type {
        ReturnDescriptor::Void => "V".to_owned(),
        ReturnDescriptor::Return(fd) => fd.to_string(),
    }
}

/// Full `(args)ret` descriptor, for diagnostics.
fn full_descriptor(desc: &MethodDescriptor) -> String {
    format!("({}){}", arg_descriptor(desc), ret_descriptor(desc))
}

// === Flow-analysis input ===================================================
//
// Richer per-class data for the Java-side handle-flow analysis (`flow.rs`):
// member-level handle contracts and access flags (for the declaration lints and
// for resolving call-site contracts) plus the source file (for diagnostics).
// Built from the same parsed classes as the boundary check; per-method bytecode
// is added in a later phase.

/// A class prepared for handle-flow analysis.
#[derive(Debug, Clone)]
pub struct FlowClass {
    /// Internal binary name, e.g. `example/Flow`.
    pub internal_name: String,
    /// The `SourceFile` attribute (e.g. `Flow.java` / `Flow.kt`), if present.
    pub source_file: Option<String>,
    pub fields: Vec<FlowField>,
    pub methods: Vec<FlowMethod>,
    /// `@SuppressJni` categories declared on the class itself (silences those
    /// categories everywhere in the class).
    pub suppressed: Vec<String>,
}

/// A field, with its handle contract and access (for the exposure/field checks).
#[derive(Debug, Clone)]
pub struct FlowField {
    pub name: String,
    /// JVM field descriptor, e.g. `J`, `Ljava/lang/String;`.
    pub descriptor: String,
    pub is_public: bool,
    pub is_protected: bool,
    pub is_static: bool,
    /// The `@Owned`/`@Ref`/`@Mut` contract, if this field declares one.
    pub handle: Option<Pointer>,
    /// `@SuppressJni` categories on the field declaration.
    pub suppressed: Vec<String>,
}

/// A method, with its per-parameter / return handle contracts and access.
#[derive(Debug, Clone)]
pub struct FlowMethod {
    pub name: String,
    /// Full `(args)ret` descriptor, e.g. `(Ljava/lang/String;)J`.
    pub descriptor: String,
    pub is_public: bool,
    pub is_protected: bool,
    pub is_static: bool,
    pub is_native: bool,
    /// Per-parameter handle contract (`None` where the slot is not a handle).
    pub params: Vec<Option<Pointer>>,
    /// Return handle contract, if the method returns a handle.
    pub ret: Option<Pointer>,
    /// The analyzable method body, if it has decoded bytecode (absent for
    /// `native`/`abstract` methods).
    pub code: Option<MethodCode>,
    /// `@SuppressJni` categories on the method declaration.
    pub suppressed: Vec<String>,
}

/// Load every class in `paths` as a [`FlowClass`] for the flow analysis. Accepts
/// the same `.class` / directory / `.jar` inputs as [`load`].
pub fn load_flow(paths: &[PathBuf]) -> Result<Vec<FlowClass>, JavaLoadError> {
    let mut out = Vec::new();
    visit_classes(paths, |class| {
        out.push(build_flow_class(class));
        Ok(())
    })?;
    Ok(out)
}

fn build_flow_class(class: &cafebabe::ClassFile) -> FlowClass {
    let fields = class
        .fields
        .iter()
        .map(|fd| FlowField {
            name: fd.name.to_string(),
            descriptor: fd.descriptor.to_string(),
            is_public: fd.access_flags.contains(FieldAccessFlags::PUBLIC),
            is_protected: fd.access_flags.contains(FieldAccessFlags::PROTECTED),
            is_static: fd.access_flags.contains(FieldAccessFlags::STATIC),
            handle: field_pointer_annotation(&fd.attributes),
            suppressed: suppress_categories(&fd.attributes),
        })
        .collect();

    let methods = class
        .methods
        .iter()
        // Skip the static initializer and compiler-synthesized methods (lambda
        // bodies, bridges): they are desugaring noise with no source-level
        // handle contract to check.
        .filter(|m| {
            m.name.as_ref() != "<clinit>"
                && !m.access_flags.contains(MethodAccessFlags::SYNTHETIC)
                && !m.access_flags.contains(MethodAccessFlags::BRIDGE)
        })
        .map(|m| {
            let anns = collect_pointer_annotations(&m.attributes);
            // Reuse `pointer_ir`, which validates the slot is a bare `long`; a
            // misannotation on any other slot is not a usable handle contract.
            let to_handle = |fd: &FieldDescriptor, ann: &PtrAnn| match pointer_ir(fd, ann) {
                IrType::Pointer(p) => Some(p),
                _ => None,
            };
            let params: Vec<Option<Pointer>> = m
                .descriptor
                .parameters
                .iter()
                .enumerate()
                .map(|(i, fd)| anns.params.get(&i).and_then(|ann| to_handle(fd, ann)))
                .collect();
            let ret = match &m.descriptor.return_type {
                ReturnDescriptor::Return(fd) => {
                    anns.ret.as_ref().and_then(|ann| to_handle(fd, ann))
                }
                ReturnDescriptor::Void => None,
            };
            let is_static = m.access_flags.contains(MethodAccessFlags::STATIC);
            let code = m.attributes.iter().find_map(|a| match &a.data {
                AttributeData::Code(cd) => lower_code(cd, m, is_static, &params),
                _ => None,
            });
            FlowMethod {
                name: m.name.to_string(),
                descriptor: full_descriptor(&m.descriptor),
                is_public: m.access_flags.contains(MethodAccessFlags::PUBLIC),
                is_protected: m.access_flags.contains(MethodAccessFlags::PROTECTED),
                is_static,
                is_native: m.access_flags.contains(MethodAccessFlags::NATIVE),
                params,
                ret,
                code,
                suppressed: suppress_categories(&m.attributes),
            }
        })
        .collect();

    FlowClass {
        internal_name: class.this_class.to_string(),
        source_file: source_file(class),
        fields,
        methods,
        suppressed: suppress_categories(&class.attributes),
    }
}

fn source_file(class: &cafebabe::ClassFile) -> Option<String> {
    class.attributes.iter().find_map(|a| match &a.data {
        AttributeData::SourceFile(s) => Some(s.to_string()),
        _ => None,
    })
}

/// Lower a method's decoded `Code` into the owned [`MethodCode`] the flow
/// analysis walks. `None` if the bytecode was not decoded (e.g. parsing was
/// configured to skip it).
fn lower_code(
    cd: &cafebabe::attributes::CodeData,
    m: &cafebabe::MethodInfo,
    is_static: bool,
    params: &[Option<Pointer>],
) -> Option<MethodCode> {
    let bytecode = cd.bytecode.as_ref()?;

    // Line numbers: (start_pc -> line), looked up by greatest start_pc <= offset.
    let mut lines: Vec<(u32, u32)> = Vec::new();
    for a in &cd.attributes {
        if let AttributeData::LineNumberTable(entries) = &a.data {
            lines.extend(
                entries
                    .iter()
                    .map(|e| (u32::from(e.start_pc), u32::from(e.line_number))),
            );
        }
    }
    lines.sort_unstable_by_key(|(pc, _)| *pc);
    let line_at = |off: u32| -> Option<u32> {
        lines
            .iter()
            .rev()
            .find(|(pc, _)| *pc <= off)
            .map(|(_, l)| *l)
    };

    let insns = bytecode
        .opcodes
        .iter()
        .map(|(off, op)| {
            let offset = *off as u32;
            Insn {
                offset,
                line: line_at(offset),
                op: classify_op(op, offset),
            }
        })
        .collect();

    let exceptions = cd
        .exception_table
        .iter()
        .map(|e| ExceptionRange {
            start: u32::from(e.start_pc),
            end: u32::from(e.end_pc),
            handler: u32::from(e.handler_pc),
        })
        .collect();

    // Local variable names — present only when compiled with `-g`/`-g:vars`.
    let mut local_names = Vec::new();
    for a in &cd.attributes {
        if let AttributeData::LocalVariableTable(entries) = &a.data {
            local_names.extend(entries.iter().map(|e| LocalName {
                slot: e.index,
                start: u32::from(e.start_pc),
                end: u32::from(e.start_pc) + u32::from(e.length),
                name: e.name.to_string(),
            }));
        }
    }

    // Local `@Ref`/`@Mut`/`@Owned` annotations (localvar_target type annotations
    // inside the `Code` attribute).
    let mut local_handles = Vec::new();
    for a in &cd.attributes {
        let tas: &[TypeAnnotation] = match &a.data {
            AttributeData::RuntimeInvisibleTypeAnnotations(v)
            | AttributeData::RuntimeVisibleTypeAnnotations(v) => v,
            _ => continue,
        };
        for ta in tas {
            if !ta.target_path.is_empty() {
                continue;
            }
            let TypeAnnotationTarget::LocalVar(entries) = &ta.target_type else {
                continue;
            };
            let Some(kind) = pointer_kind(&annotation_class(&ta.annotation.type_descriptor)) else {
                continue;
            };
            let (rust_type, nullable) = read_members(&ta.annotation);
            local_handles.extend(entries.iter().map(|e| LocalHandleAnn {
                slot: e.index,
                start: u32::from(e.start_pc),
                end: u32::from(e.start_pc) + u32::from(e.length),
                ptr: Pointer {
                    kind,
                    rust_type: rust_type.clone(),
                    nullable,
                },
            }));
        }
    }

    // Parameter slot layout: `this` at slot 0 for instance methods, then each
    // parameter in turn (a long/double occupies two local slots).
    let mut param_infos = Vec::with_capacity(params.len());
    let mut slot: u16 = u16::from(!is_static);
    for (i, fd) in m.descriptor.parameters.iter().enumerate() {
        let wide =
            fd.dimensions == 0 && matches!(fd.field_type, FieldType::Long | FieldType::Double);
        param_infos.push(ParamInfo {
            slot,
            wide,
            handle: params.get(i).cloned().flatten(),
        });
        slot += if wide { 2 } else { 1 };
    }

    Some(MethodCode {
        max_locals: cd.max_locals,
        insns,
        exceptions,
        local_names,
        local_handles,
        params: param_infos,
    })
}

/// Classify one decoded opcode into the owned [`Op`], resolving branch targets to
/// absolute offsets. Opcodes irrelevant to handle-flow collapse into [`Op::Other`]
/// carrying just their value-level stack effect, so the abstract stack stays
/// balanced.
fn classify_op(op: &Opcode, offset: u32) -> Op {
    use Opcode as O;

    let target = |j: i32| -> u32 { (i64::from(offset) + i64::from(j)) as u32 };
    let member = |m: &cafebabe::constant_pool::MemberRef| MemberRef {
        class: m.class_name.to_string(),
        name: m.name_and_type.name.to_string(),
        descriptor: m.name_and_type.descriptor.to_string(),
    };
    let field_wide = |m: &cafebabe::constant_pool::MemberRef| {
        matches!(m.name_and_type.descriptor.as_ref(), "J" | "D")
    };
    let invoke = |m: &cafebabe::constant_pool::MemberRef, is_static: bool| {
        let target = member(m);
        let (arg_widths, ret) = code::parse_method_descriptor(&target.descriptor);
        Op::Invoke {
            target,
            is_static,
            arg_widths,
            ret,
        }
    };

    match op {
        // --- long constants & arithmetic (provenance-relevant) ---
        O::Lconst0 => Op::LongConst { zero: true },
        O::Lconst1 => Op::LongConst { zero: false },
        O::Ldc2W(cafebabe::constant_pool::Loadable::LiteralConstant(
            cafebabe::constant_pool::LiteralConstant::Long(v),
        )) => Op::LongConst { zero: *v == 0 },
        // `ldc2_w` of a double is a wide non-handle value.
        O::Ldc2W(_) => Op::Other {
            pops: 0,
            pushes: vec![true],
        },
        O::Ladd
        | O::Lsub
        | O::Lmul
        | O::Ldiv
        | O::Lrem
        | O::Land
        | O::Lor
        | O::Lxor
        | O::Lshl
        | O::Lshr
        | O::Lushr => Op::LongCompute { pops: 2 },
        O::Lneg | O::I2l | O::F2l | O::D2l => Op::LongCompute { pops: 1 },
        O::Lcmp => Op::LongCmp,

        // --- loads / stores ---
        O::Lload(n) => Op::LoadLong(*n),
        O::Lstore(n) => Op::StoreLong(*n),
        O::Aload(n) => Op::LoadRef(*n),
        O::Astore(n) => Op::StoreRef(*n),

        // --- fields ---
        O::Getfield(m) => Op::GetField {
            field: member(m),
            wide: field_wide(m),
        },
        O::Putfield(m) => Op::PutField {
            field: member(m),
            wide: field_wide(m),
        },
        O::Getstatic(m) => Op::GetStatic {
            field: member(m),
            wide: field_wide(m),
        },
        O::Putstatic(m) => Op::PutStatic {
            field: member(m),
            wide: field_wide(m),
        },

        // --- calls ---
        O::Invokestatic(m) => invoke(m, true),
        O::Invokevirtual(m) | O::Invokespecial(m) => invoke(m, false),
        O::Invokeinterface(m, _) => invoke(m, false),
        // invokedynamic (lambdas / string concat) is not modelled — out of scope.
        O::Invokedynamic(_) => Op::Other {
            pops: 0,
            pushes: vec![],
        },

        // --- control flow ---
        O::Goto(j) => Op::Goto(target(*j)),
        O::Ifeq(j) => Op::Branch {
            target: target(*j),
            pops: 1,
            kind: code::BranchKind::IfEq,
        },
        O::Ifne(j) => Op::Branch {
            target: target(*j),
            pops: 1,
            kind: code::BranchKind::IfNe,
        },
        O::Iflt(j) | O::Ifge(j) | O::Ifgt(j) | O::Ifle(j) | O::Ifnull(j) | O::Ifnonnull(j) => {
            Op::Branch {
                target: target(*j),
                pops: 1,
                kind: code::BranchKind::Other,
            }
        }
        O::IfIcmpeq(j)
        | O::IfIcmpne(j)
        | O::IfIcmplt(j)
        | O::IfIcmpge(j)
        | O::IfIcmpgt(j)
        | O::IfIcmple(j)
        | O::IfAcmpeq(j)
        | O::IfAcmpne(j) => Op::Branch {
            target: target(*j),
            pops: 2,
            kind: code::BranchKind::Other,
        },
        O::Tableswitch(rt) => {
            let mut targets = vec![target(rt.default)];
            targets.extend(rt.jumps.iter().map(|j| target(*j)));
            Op::Switch { targets }
        }
        O::Lookupswitch(lt) => {
            let mut targets = vec![target(lt.default)];
            targets.extend(lt.match_offsets.iter().map(|(_, j)| target(*j)));
            Op::Switch { targets }
        }
        O::Ireturn | O::Lreturn | O::Freturn | O::Dreturn | O::Areturn => Op::Return { pops: 1 },
        O::Return => Op::Return { pops: 0 },
        O::Athrow => Op::Athrow,

        // --- stack shuffles ---
        O::Dup => Op::Dup,
        O::Dup2 => Op::Dup2,
        O::DupX1 => Op::DupX1,
        O::DupX2 => Op::DupX2,
        O::Dup2X1 => Op::Dup2X1,
        O::Dup2X2 => Op::Dup2X2,
        O::Pop => Op::Pop,
        O::Pop2 => Op::Pop2,
        O::Swap => Op::Swap,

        // --- everything else: value-level stack effect only ---
        O::Nop | O::Breakpoint | O::Impdep1 | O::Impdep2 | O::Iinc(..) | O::Ret(_) => Op::Other {
            pops: 0,
            pushes: vec![],
        },
        O::AconstNull
        | O::IconstM1
        | O::Iconst0
        | O::Iconst1
        | O::Iconst2
        | O::Iconst3
        | O::Iconst4
        | O::Iconst5
        | O::Fconst0
        | O::Fconst1
        | O::Fconst2
        | O::Bipush(_)
        | O::Sipush(_)
        | O::Iload(_)
        | O::Fload(_)
        | O::Ldc(_)
        | O::LdcW(_)
        | O::Jsr(_)
        | O::New(_) => Op::Other {
            pops: 0,
            pushes: vec![false],
        },
        O::Dconst0 | O::Dconst1 | O::Dload(_) => Op::Other {
            pops: 0,
            pushes: vec![true],
        },
        O::Istore(_) | O::Fstore(_) | O::Dstore(_) | O::Monitorenter | O::Monitorexit => {
            Op::Other {
                pops: 1,
                pushes: vec![],
            }
        }
        O::Ineg
        | O::I2f
        | O::I2b
        | O::I2c
        | O::I2s
        | O::L2i
        | O::L2f
        | O::F2i
        | O::Fneg
        | O::D2i
        | O::D2f
        | O::Arraylength
        | O::Checkcast(_)
        | O::Instanceof(_)
        | O::Newarray(_)
        | O::Anewarray(_) => Op::Other {
            pops: 1,
            pushes: vec![false],
        },
        O::I2d | O::L2d | O::F2d | O::Dneg => Op::Other {
            pops: 1,
            pushes: vec![true],
        },
        O::Iadd
        | O::Isub
        | O::Imul
        | O::Idiv
        | O::Irem
        | O::Iand
        | O::Ior
        | O::Ixor
        | O::Ishl
        | O::Ishr
        | O::Iushr
        | O::Fadd
        | O::Fsub
        | O::Fmul
        | O::Fdiv
        | O::Frem
        | O::Fcmpg
        | O::Fcmpl
        | O::Dcmpg
        | O::Dcmpl
        | O::Iaload
        | O::Baload
        | O::Caload
        | O::Saload
        | O::Faload
        | O::Aaload => Op::Other {
            pops: 2,
            pushes: vec![false],
        },
        O::Dadd | O::Dsub | O::Dmul | O::Ddiv | O::Drem | O::Daload | O::Laload => Op::Other {
            pops: 2,
            pushes: vec![true],
        },
        O::Iastore
        | O::Bastore
        | O::Castore
        | O::Sastore
        | O::Fastore
        | O::Aastore
        | O::Dastore
        | O::Lastore => Op::Other {
            pops: 3,
            pushes: vec![],
        },
        O::Multianewarray(_, dims) => Op::Other {
            pops: *dims,
            pushes: vec![false],
        },
    }
}
