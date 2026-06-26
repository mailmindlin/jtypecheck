//! Deliberately-wrong macro-based native methods, paired with
//! `example/java/example/IncorrectMacros.java`. Each is implemented via one of
//! the jni 0.22.4 macros and isolates a single diagnostic, proving the checker
//! validates macro-declared natives the same way it does hand-written exports.
//!
//! Like `tests/fixtures/incorrect/wrong.rs`, this file is parsed by the `syn`
//! loader but never compiled, so the impls are stubs.

use jni::{
    EnvUnowned, NativeMethod, bind_java_type, jni_mangle, native_method,
    objects::{JClass, JString},
    sys::jlong,
};
use jnisafe::{JOwned, JRef};

// E023: Java says @Ref, Rust (#[jni_mangle]) takes JOwned — pointer-kind mismatch.
#[jni_mangle("example.IncorrectMacros")]
pub fn kind_mismatch<'local>(
    mut _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _ptr: JOwned<Box<String>>,
) {
    unimplemented!()
}

// W003: Rust (#[jni_mangle]) returns a borrow handle (JRef). nullable=false on
// the Java side so the only finding is the borrow-return warning.
#[jni_mangle("example.IncorrectMacros")]
pub fn borrow_return<'local>(
    mut _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> JRef<'local, Box<String>> {
    unimplemented!()
}

// E024: Java @Owned("Box<String>"), Rust (native_method!) JOwned<Box<u32>> — type mismatch.
const _TYPE_MISMATCH: NativeMethod = native_method! {
    java_type = "example.IncorrectMacros",
    type_map = { unsafe JOwned<Box<u32>> => long },
    static fn type_mismatch(ptr: JOwned<Box<u32>>),
};

// E021: Java `int`, Rust (bind_java_type! native_methods) `jlong` — primitive kind mismatch.
bind_java_type! {
    pub IncorrectMacros => "example.IncorrectMacros",
    native_methods {
        static extern fn prim_mismatch(value: jlong),
    },
}
