//! Deliberately-wrong JNI exports paired with `example/java/example/Incorrect.java`.
//!
//! This file is NOT compiled — it is parsed by the `syn` Rust loader as a
//! fixture. Each function reproduces a Java native method's mangled symbol but
//! disagrees in exactly one way, so the checker emits one diagnostic per case.
//! `createWrongType` is intentionally absent (→ E001); `orphan` has no Java
//! counterpart (→ W001).

use jni::objects::{JClass, JString};
use jni::sys::{jint, jlong};
use jni::EnvUnowned;
use jnisafe::{JOwned, JRef};

// Java declares @Ref but Rust takes JOwned → pointer-kind mismatch (E023).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_kindMismatch<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _ptr: JOwned<Box<String>>,
) {
}

// Box<String> (Java) vs Box<u32> (Rust) → pointer-type mismatch (E024).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_typeMismatch<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _ptr: JRef<'local, Box<u32>>,
) {
}

// Java nullable=false but Rust Option<JRef<..>> → nullability mismatch (E025).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_nullMismatch<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _ptr: Option<JRef<'local, Box<String>>>,
) {
}

// Java String vs Rust jint → type-category mismatch (E020).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_objMismatch<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _value: jint,
) {
}

// Java int vs Rust jlong → primitive-kind mismatch (E021).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_primMismatch<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _value: jlong,
) {
}

// Java has two params, Rust has one (after env+class) → arity mismatch (E010).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_arityMismatch<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _value: JString<'local>,
) {
}

// Java is an instance method, but Rust takes JClass → receiver mismatch (E002).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_recvMismatch<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
}

// No Java native method named `orphan` → orphan-export warning (W001).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_orphan<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
}

// Java annotates an `int` slot with @Ref; a handle can't fit there (E026).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_badSlot<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _handle: jint,
) {
}

// Returns a borrow handle (JRef) to Java — borrowed lifetime escapes (W003).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_borrowReturn<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> JRef<'local, Box<String>> {
    unimplemented!()
}

// Looks like an export but is missing #[no_mangle] → not actually exported (W002).
pub extern "system" fn Java_example_Incorrect_notExported<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
}

// Too few parameters: no room for JNIEnv + receiver (E004).
#[no_mangle]
pub extern "system" fn Java_example_Incorrect_tooFewParams<'local>(_env: EnvUnowned<'local>) {}
