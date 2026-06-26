//! Java front-end: read `.class` / `.jar` with cafebabe and lower native
//! methods (plus their `@Ref`/`@Mut`/`@Owned` type annotations) into [`Signature`]s.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use cafebabe::attributes::{
    AnnotationElementValue, AttributeData, TypeAnnotation, TypeAnnotationTarget,
};
use cafebabe::descriptors::{FieldDescriptor, FieldType, MethodDescriptor, ReturnDescriptor};
use cafebabe::{FieldAccessFlags, MethodAccessFlags, parse_class};

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
