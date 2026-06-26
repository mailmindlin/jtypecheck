//! `example.Overloaded` implemented with two overloaded `native_method! { }`
//! natives that share the Java name `combine`.
//!
//! Because the natives share a name, the JVM resolves each by its long
//! `Java_example_Overloaded_combine__<args>` symbol. The checker's
//! `resolve_overloads` pass re-mangles these macro natives to the same long form
//! (`__II` and `__Ljava_lang_String_2…`), so each pairs with its Java
//! declaration with no collision.
//!
//! The `name = "combine"` + `fn = …` property form binds each overload to a
//! distinct Rust impl — Rust can't have two free fns named `combine`, and the
//! inline `fn combine(..)` shorthand would default the impl path to `combine`.

use jni::{
    Env, NativeMethod,
    errors::Error,
    native_method,
    objects::{JClass, JString},
    sys::jint,
};

/// Registration table for the two `combine` overloads.
pub const METHODS: &[NativeMethod] = &[
    native_method! {
        java_type = "example.Overloaded",
        name = "combine",
        static = true,
        sig = (a: jint, b: jint) -> jint,
        fn = combine_ii,
    },
    native_method! {
        java_type = "example.Overloaded",
        name = "combine",
        static = true,
        sig = (a: JString, b: JString) -> jint,
        fn = combine_ss,
    },
];

fn combine_ii<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    a: jint,
    b: jint,
) -> Result<jint, Error> {
    Ok(a + b)
}

fn combine_ss<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    a: JString<'local>,
    b: JString<'local>,
) -> Result<jint, Error> {
    let a = a.mutf8_chars(env)?.to_string();
    let b = b.mutf8_chars(env)?.to_string();
    Ok((a.len() + b.len()) as jint)
}
