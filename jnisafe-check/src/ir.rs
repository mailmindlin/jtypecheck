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
            IrType::Unsupported(s) => format!("unsupported({s})"),
        }
    }
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
}
