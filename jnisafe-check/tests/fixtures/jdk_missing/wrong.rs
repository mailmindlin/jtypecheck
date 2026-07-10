//! Parse-only fixture: a `bind_java_type!` binding to a method that exists on
//! neither `java.nio.ByteBuffer` nor any of its supertypes. Once the class is
//! resolved from the JDK (PR-A), the missing method is a hard **E040** (not the
//! "class not provided" W004). Paired with the e2e test
//! `jdk_missing_member_reports_e040`.
//!
//! NOT compiled — parsed by the `syn` Rust loader as a fixture.

use jni::{bind_java_type, sys::jint};

bind_java_type! {
    pub Buf => "java.nio.ByteBuffer",
    methods {
        // E040: no such method anywhere in ByteBuffer's hierarchy.
        fn ghost_method() -> jint,
    },
}
