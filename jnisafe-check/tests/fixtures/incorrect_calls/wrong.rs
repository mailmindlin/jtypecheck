//! Deliberately-wrong Rust→Java bindings, paired with
//! `example/java/example/IncorrectCalls.java`. Parse-only (not compiled): the
//! checker reads the `bind_java_type!` `methods`/`fields`/`constructors` clauses
//! and verifies each named member against the loaded class, so every entry here
//! isolates one reverse-direction diagnostic.
//!
//! Expected: E040 (missing method), E041 (method signature mismatch),
//! E042 (missing field), E043 (field type mismatch), E044 (no such constructor),
//! and W004 (a binding to a class that is never passed to `--java`).

use jni::{bind_java_type, objects::JString, sys::jint};

bind_java_type! {
    pub IncorrectCalls => "example.IncorrectCalls",
    methods {
        // E040: there is no `ghostMethod` on the class.
        static fn ghost_method() -> jint,
        // E041: `realMethod` exists but takes `(int) -> int`, not `(int, int) -> int`.
        static fn real_method(a: jint, b: jint) -> jint,
    },
    fields {
        // E042: there is no `missingField`.
        static missing_field: jint,
        // E043: `instanceValue` is an `int`, not a `String`.
        instance_value: JString,
    },
    constructors {
        // E044: only a `(int)` constructor exists.
        fn new(a: jint, b: jint),
    },
}

bind_java_type! {
    // W004: this class is never provided to `--java`, so it can't be verified.
    pub Elsewhere => "example.NotLoadedClass",
    methods {
        static fn whatever() -> jint,
    },
}
