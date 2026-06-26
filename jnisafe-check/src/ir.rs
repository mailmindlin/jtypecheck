//! The intermediate representation that both loaders emit and the checker
//! compares. Nothing here depends on `syn` or `cafebabe` — it is the neutral
//! meeting point between the Rust and Java front-ends.

use std::path::PathBuf;

use serde::Serialize;

/// A JVM/JNI primitive type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Primitive {
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
}

/// The flavour of a Rust smart-pointer handle, mirrored by the Java annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PointerKind {
    /// `@Ref` / `JRef`
    Ref,
    /// `@Mut` / `JMut`
    Mut,
    /// `@Owned` / `JOwned`
    Owned,
}

impl PointerKind {
    /// The Java annotation spelling (for diagnostics).
    pub fn annotation(self) -> &'static str {
        match self {
            PointerKind::Ref => "@Ref",
            PointerKind::Mut => "@Mut",
            PointerKind::Owned => "@Owned",
        }
    }

    /// The Rust wrapper spelling (for diagnostics).
    pub fn wrapper(self) -> &'static str {
        match self {
            PointerKind::Ref => "JRef",
            PointerKind::Mut => "JMut",
            PointerKind::Owned => "JOwned",
        }
    }
}

/// An annotated `long` carrying a Rust pointer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pointer {
    pub kind: PointerKind,
    /// The Rust pointee type, normalized (e.g. `Box<String>`).
    pub rust_type: String,
    /// `true` when null is representable (`@Ref` default / Rust `Option<JRef<..>>`).
    pub nullable: bool,
}

/// A single parameter or return type, in the shared vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IrType {
    Void,
    Primitive(Primitive),
    /// A Java object, keyed by its internal binary name, e.g. `java/lang/String`.
    JavaObject {
        class: String,
    },
    Pointer(Pointer),
    /// A pointer annotation (`@Ref`/`@Mut`/`@Owned`) placed on a slot that is
    /// not a bare `long` — the handle cannot live there. `java_desc` is the
    /// human-readable Java type of the slot (e.g. `int`, `java.lang.String`);
    /// `narrow_int` is `true` for integer slots narrower than `long`
    /// (`int`/`short`/`byte`/`char`), which would truncate the address.
    Misannotated {
        ann_kind: PointerKind,
        java_desc: String,
        narrow_int: bool,
    },
    /// A type the loader recognised syntactically but cannot map yet.
    Unsupported(String),
}

impl IrType {
    /// Human-friendly rendering for diagnostics.
    pub fn describe(&self) -> String {
        match self {
            IrType::Void => "void".to_owned(),
            IrType::Primitive(p) => format!("{p:?}").to_lowercase(),
            IrType::JavaObject { class } => class.replace('/', "."),
            IrType::Pointer(p) => {
                let null = if p.nullable { ", nullable" } else { "" };
                format!("{}({}{})", p.kind.annotation(), p.rust_type, null)
            }
            IrType::Misannotated {
                ann_kind,
                java_desc,
                ..
            } => format!("{} on non-`long` ({java_desc})", ann_kind.annotation()),
            IrType::Unsupported(s) => format!("unsupported({s})"),
        }
    }

    /// The raw JVM field descriptor for this type (e.g. `I`, `Ljava/lang/String;`,
    /// `[I`), or `None` when the type can't be encoded as one. Mirrors
    /// `java_loader`'s `FieldDescriptor::to_string()` exactly so the two sides
    /// agree on overload-mangling and signature comparison — note `JOwned`/`JRef`/
    /// `JMut` handles are bare `long`s on the wire, hence `J`. The result is the
    /// *unescaped* form; `mangle::mangle` applies the `_1`/`_2`/`_3` escaping.
    pub fn jni_field_descriptor(&self) -> Option<String> {
        Some(match self {
            IrType::Primitive(p) => match p {
                Primitive::Boolean => "Z",
                Primitive::Byte => "B",
                Primitive::Char => "C",
                Primitive::Short => "S",
                Primitive::Int => "I",
                Primitive::Long => "J",
                Primitive::Float => "F",
                Primitive::Double => "D",
            }
            .to_owned(),
            // Arrays are stored as their full descriptor already (e.g. `[I`,
            // `[Ljava/lang/String;`); plain objects as the internal name.
            IrType::JavaObject { class } if class.starts_with('[') => class.clone(),
            IrType::JavaObject { class } => format!("L{class};"),
            // A handle is a `long` at the FFI boundary.
            IrType::Pointer(_) => "J".to_owned(),
            IrType::Void | IrType::Misannotated { .. } | IrType::Unsupported(_) => return None,
        })
    }
}

/// Concatenated parameter descriptors (no parens, no return), matching
/// `java_loader::arg_descriptor`. `None` if any parameter can't be encoded.
pub fn args_descriptor(params: &[IrType]) -> Option<String> {
    let mut out = String::new();
    for p in params {
        out.push_str(&p.jni_field_descriptor()?);
    }
    Some(out)
}

/// A structural problem with a Rust `Java_*` function that stops it from being
/// a valid JNI export. Discovered by the Rust loader, reported by the checker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RustExportProblem {
    /// Named `Java_*` but missing `#[no_mangle]` / not `extern "system"`/`"C"`.
    NotExported,
    /// Fewer than two leading parameters (no room for `JNIEnv` + receiver).
    TooFewParams,
}

/// What the Rust function takes as its second argument (the JNI receiver).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Receiver {
    /// `JClass` — expected for `static` native methods.
    Class,
    /// `JObject` — expected for instance native methods.
    Object,
    Unknown,
}

/// Identity used to pair a Java native method with a Rust export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MethodKey {
    /// The mangled JNI symbol, e.g. `Java_example_Correct_create`.
    pub symbol: String,
    /// Internal binary class name, e.g. `example/Correct`.
    pub java_class: String,
    pub java_method: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SrcLoc {
    pub file: PathBuf,
    pub symbol: String,
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JavaLoc {
    pub class: String,
    pub method: String,
    pub descriptor: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Origin {
    pub rust: Option<SrcLoc>,
    pub java: Option<JavaLoc>,
}

/// A normalized signature. The Java loader fills `is_static`; the Rust loader
/// fills `receiver`. The remaining fields are produced symmetrically by both.
#[derive(Debug, Clone, Serialize)]
pub struct Signature {
    pub key: MethodKey,
    /// Meaningful on the Java side (from `ACC_STATIC`).
    pub is_static: bool,
    /// Meaningful on the Rust side (`JClass` vs `JObject`).
    pub receiver: Receiver,
    /// Java-aligned parameters (the Rust side already dropped env + receiver).
    pub params: Vec<IrType>,
    pub ret: IrType,
    pub origin: Origin,
    /// Set by the Rust loader when a `Java_*` fn is structurally not a valid
    /// export; always `None` for Java signatures and well-formed Rust exports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_problem: Option<RustExportProblem>,
}

/// Which kind of Java member a [`JavaRef`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JavaRefKind {
    Method,
    Constructor,
    Field,
}

impl JavaRefKind {
    /// Human noun for diagnostics.
    pub fn noun(self) -> &'static str {
        match self {
            JavaRefKind::Method => "method",
            JavaRefKind::Constructor => "constructor",
            JavaRefKind::Field => "field",
        }
    }
}

/// A `bind_java_type!` `methods`/`fields`/`constructors` entry: the Rust side
/// asserts the bound Java class has this member and *calls* it (the Rust→Java
/// direction, the reverse of a native method). The checker verifies it against
/// the loaded [`JavaClassModel`].
#[derive(Debug, Clone, Serialize)]
pub struct JavaRef {
    /// Internal binary class name, e.g. `example/BindTypeExample`.
    pub class_internal: String,
    pub kind: JavaRefKind,
    /// Java member name (camel-cased / overridden; `<init>` for a constructor).
    pub java_name: String,
    pub is_static: bool,
    /// Method / constructor parameters (empty for a field).
    pub params: Vec<IrType>,
    /// Method return type (`Void` for a constructor; unused for a field).
    pub ret: IrType,
    /// Field type — `Some` only for [`JavaRefKind::Field`].
    pub field_ty: Option<IrType>,
    pub origin: SrcLoc,
}

/// The callable surface of a Java class, used to verify [`JavaRef`]s. Built by
/// the Java loader from all (not just native) methods, fields, and `<init>`s.
#[derive(Debug, Clone, Serialize)]
pub struct JavaClassModel {
    /// Internal binary name, e.g. `example/BindTypeExample`.
    pub internal_name: String,
    pub methods: Vec<JavaMethodSig>,
    pub fields: Vec<JavaFieldSig>,
    /// `<init>` argument descriptors, e.g. `"I"` for a `(int)` constructor.
    pub constructors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JavaMethodSig {
    pub name: String,
    pub is_static: bool,
    /// Concatenated parameter descriptors, e.g. `"ILjava/lang/String;"`.
    pub arg_descriptor: String,
    /// Return descriptor, e.g. `"I"` or `"V"`.
    pub ret_descriptor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JavaFieldSig {
    pub name: String,
    pub is_static: bool,
    /// Field type descriptor, e.g. `"I"`, `"Ljava/lang/String;"`.
    pub descriptor: String,
    /// The `@Ref`/`@Mut`/`@Owned` annotation on the field's type, if any —
    /// present when the `long` field stores a jnisafe handle. `None` for an
    /// unannotated field.
    pub annotation: Option<Pointer>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(class: &str) -> IrType {
        IrType::JavaObject {
            class: class.to_owned(),
        }
    }

    #[test]
    fn field_descriptors_match_jvm_form() {
        assert_eq!(
            IrType::Primitive(Primitive::Int)
                .jni_field_descriptor()
                .as_deref(),
            Some("I")
        );
        assert_eq!(
            IrType::Primitive(Primitive::Long)
                .jni_field_descriptor()
                .as_deref(),
            Some("J")
        );
        assert_eq!(
            obj("java/lang/String").jni_field_descriptor().as_deref(),
            Some("Ljava/lang/String;")
        );
        // Arrays are stored as full descriptors and pass through verbatim.
        assert_eq!(obj("[I").jni_field_descriptor().as_deref(), Some("[I"));
        assert_eq!(
            obj("[Ljava/lang/String;").jni_field_descriptor().as_deref(),
            Some("[Ljava/lang/String;")
        );
        // A handle is a bare `long`.
        let owned = IrType::Pointer(Pointer {
            kind: PointerKind::Owned,
            rust_type: "Box<String>".to_owned(),
            nullable: false,
        });
        assert_eq!(owned.jni_field_descriptor().as_deref(), Some("J"));
        // Unencodable types.
        assert_eq!(IrType::Void.jni_field_descriptor(), None);
        assert_eq!(
            IrType::Unsupported("Foo".to_owned()).jni_field_descriptor(),
            None
        );
    }

    #[test]
    fn args_descriptor_concatenates_and_short_circuits() {
        use Primitive::*;
        assert_eq!(
            args_descriptor(&[IrType::Primitive(Int), IrType::Primitive(Int)]).as_deref(),
            Some("II")
        );
        assert_eq!(
            args_descriptor(&[obj("java/lang/String"), obj("java/lang/String")]).as_deref(),
            Some("Ljava/lang/String;Ljava/lang/String;")
        );
        assert_eq!(args_descriptor(&[]).as_deref(), Some(""));
        // Any unencodable param poisons the whole descriptor.
        assert_eq!(
            args_descriptor(&[IrType::Primitive(Int), IrType::Unsupported("x".to_owned())]),
            None
        );
    }
}
