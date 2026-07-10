//! Deliberately-*correct* JNI exports paired with `tests/fixtures/arrays/Arrays.java`.
//!
//! This file is NOT compiled — it is parsed by the `syn` Rust loader as a
//! fixture. Every method binds an array-typed Java native through the generic
//! jni array wrappers, which the Rust loader lowers via the element's generic
//! argument: `JPrimitiveArray<jint>` → `[I`, `JObjectArray<JString>` →
//! `[Ljava/lang/String;`, bare `JObjectArray` → `[Ljava/lang/Object;`, and a
//! nested `JObjectArray<JByteArray>` → `[[B`. The e2e test
//! `array_wrappers_match_their_java_declarations` asserts the clean match.

use jni::objects::{JByteArray, JClass, JObjectArray, JPrimitiveArray, JString};
use jni::sys::{jbyte, jint};
use jni::EnvUnowned;

// int[] → JPrimitiveArray<jint>  (`[I`).
#[no_mangle]
pub extern "system" fn Java_example_Arrays_sumInts<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _values: JPrimitiveArray<'local, jint>,
) -> jint {
    0
}

// return byte[] → JPrimitiveArray<jbyte>  (`[B`).
#[no_mangle]
pub extern "system" fn Java_example_Arrays_makeBytes<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _len: jint,
) -> JPrimitiveArray<'local, jbyte> {
    unimplemented!()
}

// String[] → JObjectArray<JString>  (`[Ljava/lang/String;`).
#[no_mangle]
pub extern "system" fn Java_example_Arrays_countStrings<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _names: JObjectArray<'local, JString<'local>>,
) -> jint {
    0
}

// Object[] → bare JObjectArray, default JObject element  (`[Ljava/lang/Object;`).
#[no_mangle]
pub extern "system" fn Java_example_Arrays_inspect<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _items: JObjectArray<'local>,
) {
}

// byte[][] → nested JObjectArray<JByteArray>  (`[[B`).
#[no_mangle]
pub extern "system" fn Java_example_Arrays_totalLen<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _chunks: JObjectArray<'local, JByteArray<'local>>,
) -> jint {
    0
}
