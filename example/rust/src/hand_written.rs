//! `example.HandWritten` — the baseline: native methods implemented as
//! hand-written `#[unsafe(no_mangle)] Java_*` exports, with no macro help.
//!
//! [`crate::mangle`] implements the *same* create/tryGet/get/set/drop contract
//! with `#[jni_mangle]`; diffing the two shows that the attribute only renames
//! the export and sets the ABI — the `JOwned`/`JRef`/`JMut` handle types and
//! the bodies are identical.

use std::borrow::Cow;

use jni::{
    EnvUnowned,
    errors::{Error, ThrowRuntimeExAndDefault},
    objects::{JClass, JString},
};
use jnisafe::{JMut, JOwned, JRef};

#[unsafe(no_mangle)]
pub extern "system" fn Java_example_HandWritten_create<'local>(
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
pub extern "system" fn Java_example_HandWritten_tryGet<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    ptr: Option<JRef<'local, Box<String>>>,
) -> JString<'local> {
    env.with_env(|env| -> Result<_, Error> {
        match ptr {
            Some(ptr) => Ok(JString::new(env, &*ptr)?),
            None => Ok(JString::null()),
        }
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_example_HandWritten_get<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    ptr: JRef<'local, Box<String>>,
) -> JString<'local> {
    env.with_env(|env| -> Result<_, Error> { JString::new(env, &*ptr) })
        .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_example_HandWritten_set<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    mut ptr: JMut<'local, Box<String>>,
    value: JString<'local>,
) {
    env.with_env(|env| -> Result<_, Error> {
        // `borrow_mut()` is the checked mutable-access path: in debug builds a
        // second concurrent mutable borrow of the same object is detected.
        let mut guard = ptr.borrow_mut();
        let s: &mut String = &mut guard;
        match value.mutf8_chars(env)?.to_str() {
            Cow::Borrowed(value) => {
                s.clear();
                s.push_str(value);
            }
            Cow::Owned(value) => {
                *s = value;
            }
        }
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_example_HandWritten_drop<'local>(
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
