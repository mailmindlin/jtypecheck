//! `example.BindType` implemented with jni's `bind_java_type! { }`.
//!
//! The `native_methods { … }` block declares the natives (Java→Rust); the
//! generated `BindTypeNativeInterface` trait is implemented below. As
//! with `native_method!`, the jnisafe `JOwned` handle is carried through via
//! `type_map = { unsafe JOwned<Box<String>> => long }`.
//!
//! Like [`crate::native_method`], only the owned-handle subset (`create`/`drop`)
//! of the [`crate::hand_written`] contract goes through the macro — the borrowed
//! `tryGet`/`get`/`set` need a `'local` lifetime the const context can't name.
//!
//! What's unique here is the `methods`/`fields`/`constructors` blocks: the
//! Rust→Java direction. They generate type-safe wrappers that *call into* Java,
//! and the checker verifies each named member exists on `example.BindType` with
//! the right signature (see `jnisafe-check`'s reverse-direction E040–E044
//! diagnostics). The `round_trip` native below drives those wrappers at runtime.

use jni::{
    Env, bind_java_type,
    errors::Error,
    objects::{JClass, JString},
    sys::jint,
};
use jnisafe::JOwned;

bind_java_type! {
    pub BindType => "example.BindType",
    type_map = { unsafe JOwned<Box<String>> => long },
    native_methods {
        static extern fn create(value: JString) -> JOwned<Box<String>>,
        static extern fn drop(ptr: JOwned<Box<String>>),
        // `round_trip` mangles to Java `roundTrip`; its body (below) drives the
        // Rust→Java bindings declared in the methods/fields/constructors clauses.
        static extern fn round_trip(x: jint) -> jint,
    },
    // Rust→Java calls — verified against the plain Java members of the class.
    methods {
        static fn doubled(x: jint) -> jint,
    },
    fields {
        static counter: jint,
    },
    constructors {
        fn new(value: jint),
    },
}

impl BindTypeNativeInterface for BindTypeAPI {
    type Error = Error;

    // Exercises the Rust→Java bindings generated from the `methods`/`fields`/
    // `constructors` clauses: reset the static `counter` field, call the static
    // `doubled` method, construct a `BindType` (whose Java constructor does
    // `counter += value`), then read `counter` back. With `x = 21` this returns
    // `42`, proving all three call directions executed against the JVM.
    fn round_trip<'local>(
        env: &mut Env<'local>,
        _class: JClass<'local>,
        x: jint,
    ) -> Result<jint, Self::Error> {
        BindType::set_counter(env, 0)?;
        let doubled = BindType::doubled(env, x)?;
        let _obj = BindType::new(env, doubled)?;
        BindType::counter(env)
    }

    fn create<'local>(
        env: &mut Env<'local>,
        _class: JClass<'local>,
        value: JString<'local>,
    ) -> Result<JOwned<Box<String>>, Self::Error> {
        let value = value.mutf8_chars(env)?.to_string();
        Ok(Box::new(value).into())
    }

    fn drop<'local>(
        _env: &mut Env<'local>,
        _class: JClass<'local>,
        ptr: JOwned<Box<String>>,
    ) -> Result<(), Self::Error> {
        std::mem::drop(ptr);
        Ok(())
    }
}
