//! JNI name mangling: turn a Java class + method (+ argument descriptor for
//! overloads) into the exported C symbol the Rust side must define.
//!
//! Rules (JNI spec): package separators `/` and `.` become `_`; `_` → `_1`;
//! `;` → `_2`; `[` → `_3`; any other non-ASCII-alnum char → `_0xxxx` (the
//! 4-digit hex of each UTF-16 code unit). ASCII letters/digits pass through.

/// Mangle a single name component (class binary name, method name, or argument
/// descriptor) per the JNI escaping rules.
pub fn mangle_unit(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' => out.push(c),
            '/' | '.' => out.push('_'),
            '_' => out.push_str("_1"),
            ';' => out.push_str("_2"),
            '[' => out.push_str("_3"),
            _ => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("_0{unit:04x}"));
                }
            }
        }
    }
    out
}

/// Build the expected exported symbol for a native method.
///
/// * `class_internal` — binary class name, e.g. `example/Correct`.
/// * `method` — the Java method name.
/// * `overloaded` — when true, emit the long form `..._method__<argdesc>`.
/// * `arg_descriptor` — the parameter portion of the method descriptor with no
///   surrounding parens, e.g. `Ljava/lang/String;J` (only used when overloaded).
pub fn mangle(
    class_internal: &str,
    method: &str,
    overloaded: bool,
    arg_descriptor: &str,
) -> String {
    let mut s = format!(
        "Java_{}_{}",
        mangle_unit(class_internal),
        mangle_unit(method)
    );
    if overloaded {
        s.push_str("__");
        s.push_str(&mangle_unit(arg_descriptor));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        assert_eq!(
            mangle("example/Correct", "create", false, ""),
            "Java_example_Correct_create"
        );
    }

    #[test]
    fn underscore_in_name() {
        assert_eq!(mangle("a/B", "my_method", false, ""), "Java_a_B_my_1method");
    }

    #[test]
    fn overload_long_form() {
        assert_eq!(mangle("a/B", "foo", true, "I"), "Java_a_B_foo__I");
        assert_eq!(mangle("a/B", "foo", true, "J"), "Java_a_B_foo__J");
        assert_eq!(
            mangle("a/B", "foo", true, "Ljava/lang/String;"),
            "Java_a_B_foo__Ljava_lang_String_2"
        );
    }

    #[test]
    fn array_descriptor_escapes() {
        // `[I` argument: `[` → `_3`.
        assert_eq!(mangle("a/B", "f", true, "[I"), "Java_a_B_f___3I");
    }

    #[test]
    fn unicode() {
        // U+00E9 'é' → _000e9
        assert_eq!(mangle_unit("é"), "_000e9");
    }
}
