//! Rust→Java handle-field bindings, paired with
//! `jnisafe-check/tests/fixtures/field_handles/FieldHandles.java`. Parse-only
//! (not compiled): the checker reads the `bind_java_type!` `fields { … }` clause
//! and, for each `long` field the Rust side declares as a handle, cross-checks
//! the Java `@Owned`/`@Ref`/`@Mut` annotation.
//!
//! Expected: W005 (`bare` stores a handle but is unannotated) and E045 (`wrong`
//! is annotated `@Owned("Box<u64>")`, not `Box<String>`). `cached` matches
//! cleanly and yields nothing.

use jni::bind_java_type;
use jnisafe::JOwned;

bind_java_type! {
    pub FieldHandles => "example.FieldHandles",
    type_map = { unsafe JOwned<Box<String>> => long },
    fields {
        // Clean: Java declares `@Owned("Box<String>") long cached`.
        cached: JOwned<Box<String>>,
        // W005: `long bare` stores a handle but carries no annotation.
        bare: JOwned<Box<String>>,
        // E045: Java's `@Owned("Box<u64>")` disagrees on the pointee type.
        wrong: JOwned<Box<String>>,
    },
}
