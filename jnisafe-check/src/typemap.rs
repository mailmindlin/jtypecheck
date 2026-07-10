//! Built-in correspondence between Java types and Rust JNI types, plus the
//! normalization used to compare annotated-pointer type strings.
//!
//! Both loaders lower into the same [`IrType`] vocabulary, so the matcher in
//! `check.rs` never has to know which side a type came from. A Java `String`
//! and a Rust `JString` both become `JavaObject { class: "java/lang/String" }`.

use crate::ir::{IrType, Primitive};

/// Normalize a Rust type string for comparison: drop all whitespace so that
/// `Box < String >` and `Box<String>` compare equal. The same normalization is
/// applied to the Java annotation's `value` string and to the Rust generic arg.
pub fn normalize_rust_type(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Map a cafebabe primitive `FieldType` discriminant to our [`Primitive`].
/// (Caller matches on `cafebabe::descriptors::FieldType`; we take the char it
/// would render to, which keeps this module free of a cafebabe dependency.)
pub fn primitive_from_descriptor_char(c: char) -> Option<Primitive> {
    Some(match c {
        'Z' => Primitive::Boolean,
        'B' => Primitive::Byte,
        'C' => Primitive::Char,
        'S' => Primitive::Short,
        'I' => Primitive::Int,
        'J' => Primitive::Long,
        'F' => Primitive::Float,
        'D' => Primitive::Double,
        _ => return None,
    })
}

/// Map the *name* of a Rust JNI type (the final path segment, e.g. `JString`,
/// `jint`, `JByteArray`) to an [`IrType`]. Returns `None` if unrecognised so
/// the caller can emit an `Unsupported` diagnostic rather than guessing.
pub fn rust_simple_type(name: &str) -> Option<IrType> {
    // Primitives — both the `jni::sys` aliases and bare Rust ints are accepted.
    let prim = match name {
        "jboolean" | "bool" => Some(Primitive::Boolean),
        "jbyte" | "i8" => Some(Primitive::Byte),
        "jchar" | "u16" => Some(Primitive::Char),
        "jshort" | "i16" => Some(Primitive::Short),
        "jint" | "i32" => Some(Primitive::Int),
        "jlong" | "i64" => Some(Primitive::Long),
        "jfloat" | "f32" => Some(Primitive::Float),
        "jdouble" | "f64" => Some(Primitive::Double),
        _ => None,
    };
    if let Some(p) = prim {
        return Some(IrType::Primitive(p));
    }

    // Object / reference types.
    let class = match name {
        "JString" | "jstring" => "java/lang/String",
        "JObject" | "jobject" => "java/lang/Object",
        "JClass" | "jclass" => "java/lang/Class",
        "JThrowable" | "jthrowable" => "java/lang/Throwable",
        "JByteBuffer" => "java/nio/ByteBuffer",
        // Canonical 1:1 jni-rs collection wrappers (reduce spurious E029).
        "JList" => "java/util/List",
        "JMap" => "java/util/Map",
        "JSet" => "java/util/Set",
        "JByteArray" | "jbyteArray" => "[B",
        "JBooleanArray" | "jbooleanArray" => "[Z",
        "JCharArray" | "jcharArray" => "[C",
        "JShortArray" | "jshortArray" => "[S",
        "JIntArray" | "jintArray" => "[I",
        "JLongArray" | "jlongArray" => "[J",
        "JFloatArray" | "jfloatArray" => "[F",
        "JDoubleArray" | "jdoubleArray" => "[D",
        "JObjectArray" | "jobjectArray" => "[Ljava/lang/Object;",
        _ => return None,
    };
    Some(IrType::JavaObject {
        class: class.to_owned(),
    })
}

/// Build the array type whose element is `elem`, for the generic wrappers
/// `JPrimitiveArray<T>` / `JObjectArray<E>`. The result is a [`IrType::JavaObject`]
/// whose `class` holds the full JVM array descriptor (`[I`, `[Ljava/lang/String;`,
/// or `[[B` for nested arrays). Returns `None` when `elem` has no field
/// descriptor (e.g. `void`, a handle, or an unsupported type).
pub fn array_of(elem: &IrType) -> Option<IrType> {
    let desc = elem.jni_field_descriptor()?;
    Some(IrType::JavaObject {
        class: format!("[{desc}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_whitespace() {
        assert_eq!(normalize_rust_type("Box < String >"), "Box<String>");
        assert_eq!(normalize_rust_type("Arc<Foo>"), "Arc<Foo>");
        assert_eq!(normalize_rust_type(" Vec < u8 > "), "Vec<u8>");
    }

    #[test]
    fn maps_primitives() {
        assert_eq!(
            rust_simple_type("jint"),
            Some(IrType::Primitive(Primitive::Int))
        );
        assert_eq!(
            rust_simple_type("jlong"),
            Some(IrType::Primitive(Primitive::Long))
        );
        assert_eq!(primitive_from_descriptor_char('J'), Some(Primitive::Long));
        assert_eq!(primitive_from_descriptor_char('L'), None);
    }

    #[test]
    fn maps_string_both_sides() {
        let s = IrType::JavaObject {
            class: "java/lang/String".to_owned(),
        };
        assert_eq!(rust_simple_type("JString"), Some(s.clone()));
        assert_eq!(rust_simple_type("jstring"), Some(s));
    }

    #[test]
    fn maps_collection_wrappers() {
        let obj = |c: &str| {
            Some(IrType::JavaObject {
                class: c.to_owned(),
            })
        };
        assert_eq!(rust_simple_type("JList"), obj("java/util/List"));
        assert_eq!(rust_simple_type("JMap"), obj("java/util/Map"));
        assert_eq!(rust_simple_type("JSet"), obj("java/util/Set"));
    }

    #[test]
    fn unknown_is_none() {
        assert_eq!(rust_simple_type("WeirdType"), None);
    }
}
