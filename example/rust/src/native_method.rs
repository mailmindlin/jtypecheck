//! `example.NativeMethod` implemented with jni's `native_method! { }`.
//!
//! Each method is built into a [`jni::NativeMethod`] (registered at runtime via
//! `RegisterNatives`). The owned jnisafe handle is carried through the macro
//! with `type_map = { unsafe JOwned<Box<String>> => long }`: the macro emits
//! that exact type into both the FFI wrapper and the impl below, so the handle
//! keeps its `JOwned` ergonomics while the JNI signature treats it as a `long`.
//! (The `unsafe` asserts the layout, which holds — `JOwned` is
//! `#[repr(transparent)]` over a `jlong`.)
//!
//! This implements the owned-handle subset of the [`crate::hand_written`]
//! contract (`create`/`drop`). The borrowed handles `JRef`/`JMut` carry a
//! `'local` lifetime that can't be named from the `const`-evaluated
//! `native_method!`/`bind_java_type!` context, so the borrow methods
//! (`tryGet`/`get`/`set`) are a `#[jni_mangle]` feature (see `mangle.rs`);
//! `native_method!` covers the owned handles.

// `Box<String>` is the deliberate demo handle — a Rust-owned heap object passed
// to Java — not an accidental double box, so silence clippy's box_collection
// lint on the impl signatures below (the FFI exports in `hand_written.rs` are
// exempt).
#![allow(clippy::box_collection)]

use jni::{
    Env, NativeMethod,
    errors::Error,
    native_method,
    objects::{JClass, JString},
};
use jnisafe::JOwned;

/// Registration table, registered against `example.NativeMethod` from
/// `JNI_OnLoad` (see `lib.rs`) via `env.register_native_methods(class, METHODS)`.
pub const METHODS: &[NativeMethod] = &[
    native_method! {
        java_type = "example.NativeMethod",
        type_map = { unsafe JOwned<Box<String>> => long },
        static fn create(value: JString) -> JOwned<Box<String>>,
    },
    native_method! {
        java_type = "example.NativeMethod",
        type_map = { unsafe JOwned<Box<String>> => long },
        static fn drop(ptr: JOwned<Box<String>>),
    },
];

fn create<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    value: JString<'local>,
) -> Result<JOwned<Box<String>>, Error> {
    let value = value.mutf8_chars(env)?.to_string();
    Ok(Box::new(value).into())
}

fn drop<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    ptr: JOwned<Box<String>>,
) -> Result<(), Error> {
    std::mem::drop(ptr);
    Ok(())
}
