//! Parse-only fixture: `bind_java_type!` bindings to `java.nio.ByteBuffer` using
//! members **inherited from its superclass `java.nio.Buffer`** (not declared on
//! ByteBuffer). Verifying these requires both JDK resolution (PR-A) *and*
//! inheritance-aware member lookup that walks the superclass chain (PR-B).
//! Paired with the e2e test `jdk_inherited_members_verify`, which expects a
//! clean run only once the chain is walked.
//!
//! NOT compiled — parsed by the `syn` Rust loader as a fixture.

use jni::{
    bind_java_type,
    sys::{jboolean, jint},
};

bind_java_type! {
    pub Buf => "java.nio.ByteBuffer",
    methods {
        // All three are declared on `java.nio.Buffer` and inherited (not
        // overridden) by ByteBuffer.
        fn remaining() -> jint,
        fn capacity() -> jint,
        fn has_remaining() -> jboolean,
    },
}
