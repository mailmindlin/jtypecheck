//! Deliberately-*correct* JNI exports paired with
//! `tests/fixtures/object_arrays/ObjectArrays.java`.
//!
//! NOT compiled — parsed by the `syn` Rust loader as a fixture. The point: the
//! array element type `JPose` is a *user* wrapper, not a built-in like `JString`.
//! The `bind_java_type! { pub JPose => "example.Pose" }` header below tells the
//! checker `JPose` ↔ `example/Pose`, so `JObjectArray<'local, JPose<'local>>`
//! lowers to `[Lexample/Pose;` — matching the Java `Pose[]` declarations. The
//! e2e test `object_array_of_bound_type_matches` asserts the clean match.

use jni::EnvUnowned;
use jni::objects::{JClass, JObjectArray};
use jni::sys::jint;
use jni::bind_java_type;

// Binds the wrapper type `JPose` to the Java class `example.Pose`.
bind_java_type! {
    pub JMyArrayElement => "example.MyArrayElement",
}

// return Pose[] → JObjectArray<JPose>  (`[Lexample/Pose;`).
#[no_mangle]
pub extern "system" fn Java_example_ObjectArrays_makePoses<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _n: jint,
) -> JObjectArray<'local, JMyArrayElement<'local>> {
    unimplemented!()
}

// Pose[] → JObjectArray<JPose>  (`[Lexample/Pose;`).
#[no_mangle]
pub extern "system" fn Java_example_ObjectArrays_countPoses<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _poses: JObjectArray<'local, JMyArrayElement<'local>>,
) -> jint {
    0
}
