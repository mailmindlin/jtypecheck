//! Parse-only fixture: `bind_java_type!` bindings to the JDK stdlib type
//! `java.nio.ByteBuffer`, using only members **declared on ByteBuffer itself**
//! (not inherited). Resolving these requires jnisafe-check to load the class
//! from the JDK via `--java-home` / `$JAVA_HOME` — there is no `.class` for it
//! on `--java`. Paired with the e2e test `jdk_declared_members_verify`, which
//! expects a clean run.
//!
//! NOT compiled — parsed by the `syn` Rust loader as a fixture.

use jni::{bind_java_type, objects::JByteBuffer, sys::jint};

bind_java_type! {
    pub Buf => "java.nio.ByteBuffer",
    methods {
        // `static ByteBuffer allocate(int)` — declared on ByteBuffer.
        static fn allocate(capacity: jint) -> JByteBuffer,
        // `int getInt()` — declared on ByteBuffer.
        fn get_int() -> jint,
    },
}
