//! Java front-end: read `.class` / `.jar` with cafebabe and lower native
//! methods (plus their `@Ref`/`@Mut`/`@Owned` type annotations) into [`Signature`]s.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use cafebabe::attributes::{
    AnnotationElementValue, AttributeData, TypeAnnotation, TypeAnnotationTarget,
};
use cafebabe::descriptors::{FieldDescriptor, FieldType, MethodDescriptor, ReturnDescriptor};
use cafebabe::{MethodAccessFlags, parse_class};

use crate::ir::{IrType, JavaLoc, MethodKey, Origin, Pointer, PointerKind, Receiver, Signature};
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
    for path in paths {
        if path.is_dir() {
            for entry in walkdir::WalkDir::new(path).into_iter().flatten() {
                if entry.path().extension().is_some_and(|e| e == "class") {
                    let bytes = read_file(entry.path())?;
                    load_class(&bytes, &entry.path().display().to_string(), &mut sigs)?;
                }
            }
        } else if path.extension().is_some_and(|e| e == "jar") {
            load_jar(path, &mut sigs)?;
        } else {
            let bytes = read_file(path)?;
            load_class(&bytes, &path.display().to_string(), &mut sigs)?;
        }
    }
    Ok(sigs)
}

fn read_file(path: &Path) -> Result<Vec<u8>, JavaLoadError> {
    std::fs::read(path).map_err(|e| JavaLoadError::Io(path.to_path_buf(), e))
}

fn load_jar(path: &Path, out: &mut Vec<Signature>) -> Result<(), JavaLoadError> {
    let file = std::fs::File::open(path).map_err(|e| JavaLoadError::Io(path.to_path_buf(), e))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| JavaLoadError::Jar(path.to_path_buf(), e))?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| JavaLoadError::Jar(path.to_path_buf(), e))?;
        if !entry.name().ends_with(".class") {
            continue;
        }
        let name = format!("{}!{}", path.display(), entry.name());
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| JavaLoadError::Io(path.to_path_buf(), e))?;
        load_class(&bytes, &name, out)?;
    }
    Ok(())
}

fn load_class(bytes: &[u8], source: &str, out: &mut Vec<Signature>) -> Result<(), JavaLoadError> {
    let class =
        parse_class(bytes).map_err(|e| JavaLoadError::Parse(source.to_owned(), e.to_string()))?;
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
        });
    }
    Ok(())
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
fn pointer_ir(fd: &FieldDescriptor, ann: &PtrAnn) -> IrType {
    let is_long = fd.dimensions == 0 && matches!(fd.field_type, FieldType::Long);
    if !is_long {
        return IrType::Unsupported(format!(
            "{} annotation on non-`long` ({})",
            ann.kind.annotation(),
            fd
        ));
    }
    IrType::Pointer(Pointer {
        kind: ann.kind,
        rust_type: ann.rust_type.clone(),
        nullable: ann.nullable,
    })
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

/// Full `(args)ret` descriptor, for diagnostics.
fn full_descriptor(desc: &MethodDescriptor) -> String {
    let ret = match &desc.return_type {
        ReturnDescriptor::Void => "V".to_owned(),
        ReturnDescriptor::Return(fd) => fd.to_string(),
    };
    format!("({}){}", arg_descriptor(desc), ret)
}
