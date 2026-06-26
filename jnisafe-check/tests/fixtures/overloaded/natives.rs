//! Two overloaded `native_method!` impls of `example.Overloaded.combine`,
//! paired with `example/java/example/Overloaded.java`. Parse-only (not compiled).
//!
//! The Rust loader's `resolve_overloads` pass re-mangles these same-named macro
//! natives to their long `..._combine__<args>` symbols — exactly what
//! `java_loader` emits for a class with two same-named natives — so each pairs
//! cleanly with its Java declaration (no collision). The e2e test
//! `overloaded_macro_methods_match_when_supported` asserts the clean match.

use jni::{NativeMethod, native_method, objects::JString, sys::jint};

const _COMBINE_II: NativeMethod = native_method! {
    java_type = "example.Overloaded",
    static fn combine(a: jint, b: jint) -> jint,
};

const _COMBINE_SS: NativeMethod = native_method! {
    java_type = "example.Overloaded",
    static fn combine(a: JString, b: JString) -> jint,
};
