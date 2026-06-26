//! `example.Document` — a stateful object whose Rust `Box<String>` lives in a
//! Java *field* across calls (see `../java/example/Document.java`).
//!
//! The natives are the ordinary jnisafe parameter path: identical in shape to
//! [`crate::hand_written`], with `set` replaced by `append`. What makes the
//! field usage sound is entirely on the Java side — `Document` extends the
//! `NativeObject` base class, so each call reaches Rust with the handle already
//! validated and guarded by a read/write lock (`@Ref` under the read lock,
//! `@Mut`/`@Owned` under the write lock). The Rust code needs no field access of
//! its own.

use jni::{
    EnvUnowned,
    errors::{Error, ThrowRuntimeExAndDefault},
    objects::{JClass, JString},
};
use jnisafe::{JMut, JOwned, JRef};

#[unsafe(no_mangle)]
pub extern "system" fn Java_example_Document_create<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    value: JString<'local>,
) -> JOwned<Box<String>> {
    env.with_env(|env| -> Result<_, Error> {
        let value = value.mutf8_chars(env)?.to_string();
        Ok(Box::new(value).into())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_example_Document_get<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    ptr: JRef<'local, Box<String>>,
) -> JString<'local> {
    env.with_env(|env| -> Result<_, Error> { JString::new(env, &*ptr) })
        .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_example_Document_append<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    mut ptr: JMut<'local, Box<String>>,
    suffix: JString<'local>,
) {
    env.with_env(|env| -> Result<_, Error> {
        // Read the argument before taking the mutable borrow so `env` is free.
        let suffix = suffix.mutf8_chars(env)?.to_string();
        // `borrow_mut()` is the checked mutable-access path: in debug builds a
        // second concurrent mutable borrow of the same object is detected.
        let mut guard = ptr.borrow_mut();
        guard.push_str(&suffix);
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_example_Document_drop<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    ptr: JOwned<Box<String>>,
) {
    env.with_env(|_env| -> Result<_, Error> {
        // Drop inside the closure so a panicking destructor is caught rather
        // than unwinding into the JVM.
        drop(ptr);
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}
