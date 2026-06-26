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

/// Convert a Rust `snake_case` identifier to the `lowerCamelCase` Java method
/// name `#[jni_mangle]` / `native_method!` would derive from it.
///
/// Mirrors the jni 0.22.4 rules (see `docs/macros/jni_mangle.md`):
/// * If the input contains any uppercase letter it is returned unchanged
///   (already camelCase or intentionally cased).
/// * Exactly one leading underscore is removed; any further leading underscores
///   and all trailing underscores are preserved.
/// * Each `_`-separated segment after the first has its first non-numeric
///   character uppercased (so `array_2d_foo` → `array2DFoo`). Unicode-aware.
pub fn snake_to_lower_camel(name: &str) -> String {
    if name.chars().any(|c| c.is_uppercase()) {
        return name.to_string();
    }

    let chars: Vec<char> = name.chars().collect();
    let len = chars.len();
    let lead = chars.iter().take_while(|c| **c == '_').count();
    if lead == len {
        // All underscores: drop one, keep the rest.
        return "_".repeat(len.saturating_sub(1));
    }
    let trail = chars.iter().rev().take_while(|c| **c == '_').count();
    let core: String = chars[lead..len - trail].iter().collect();

    let mut camel = String::new();
    for (i, seg) in core.split('_').enumerate() {
        if i == 0 {
            camel.push_str(seg);
        } else {
            camel.push_str(&capitalize_segment(seg));
        }
    }

    let mut out = "_".repeat(lead.saturating_sub(1));
    out.push_str(&camel);
    out.push_str(&"_".repeat(trail));
    out
}

/// Uppercase the first non-numeric character of `seg`, leaving leading digits
/// and the remainder untouched (`"2d"` → `"2D"`, `"foo"` → `"Foo"`).
fn capitalize_segment(seg: &str) -> String {
    let mut out = String::new();
    let mut capitalized = false;
    for c in seg.chars() {
        if !capitalized && !c.is_numeric() {
            out.extend(c.to_uppercase());
            capitalized = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// Turn a dotted Java class name (`com.example.Foo`, `com.example.Outer::Inner`)
/// into the internal binary name the mangler and `java_loader` use
/// (`com/example/Foo`, `com/example/Outer$Inner`).
pub fn class_dotted_to_internal(dotted: &str) -> String {
    dotted.replace("::", "$").replace('.', "/")
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

    #[test]
    fn snake_to_camel_matches_jni_doc_table() {
        // Cases lifted verbatim from jni-0.22.4 docs/macros/jni_mangle.md.
        let cases = [
            ("say_hello", "sayHello"),
            ("get_user_name", "getUserName"),
            ("_private_method", "privateMethod"),
            ("__dunder__", "_dunder__"),
            ("___priv", "__priv"),
            ("trailing_", "trailing_"),
            ("sayHello", "sayHello"),
            ("getUserName", "getUserName"),
            ("Foo_Bar", "Foo_Bar"),
            ("XMLParser", "XMLParser"),
            ("init", "init"),
            ("test_αλφα", "testΑλφα"),
            ("array_2d_foo", "array2DFoo"),
            ("test_3d", "test3D"),
        ];
        for (input, want) in cases {
            assert_eq!(
                snake_to_lower_camel(input),
                want,
                "snake→camel of {input:?}"
            );
        }
    }

    #[test]
    fn class_dotted_to_internal_forms() {
        assert_eq!(
            class_dotted_to_internal("example.Correct"),
            "example/Correct"
        );
        assert_eq!(
            class_dotted_to_internal("com.example.Foo"),
            "com/example/Foo"
        );
        assert_eq!(
            class_dotted_to_internal("com.example.Outer::Inner"),
            "com/example/Outer$Inner"
        );
    }
}
