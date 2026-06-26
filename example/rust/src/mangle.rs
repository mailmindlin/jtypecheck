//! `example.Mangle` implemented with jni's `#[jni_mangle]` attribute.
//!
//! `#[jni_mangle("example.Mangle")]` only renames the export symbol and sets the
//! `extern "system"` ABI — the function keeps its real signature, so the jnisafe
//! `JOwned`/`JRef`/`JMut` handle types are used exactly as in a hand-written
//! `Java_*` export. This implements the same contract as `hand_written.rs`; diff
//! the two to see that only the binding boilerplate differs.

use jni::{
    EnvUnowned,
    errors::{Error, ThrowRuntimeExAndDefault},
    jni_mangle,
    objects::{JClass, JString},
};
use jnisafe::{JMut, JOwned, JRef};

#[jni_mangle("example.Mangle")]
pub fn create<'local>(
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

// `try_get` (snake_case) mangles to the Java `tryGet` method. The nullable
// `@Ref` becomes `Option<JRef<..>>`, exactly as in the hand-written export.
#[jni_mangle("example.Mangle")]
pub fn try_get<'local>(
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

#[jni_mangle("example.Mangle")]
pub fn get<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    ptr: JRef<'local, Box<String>>,
) -> JString<'local> {
    env.with_env(|env| -> Result<_, Error> { JString::new(env, &*ptr) })
        .resolve::<ThrowRuntimeExAndDefault>()
}

// `set_value` (snake_case) mangles to the Java `setValue` method.
#[jni_mangle("example.Mangle")]
pub fn set_value<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    mut ptr: JMut<'local, Box<String>>,
    value: JString<'local>,
) {
    env.with_env(|env| -> Result<_, Error> {
        let mut guard = ptr.borrow_mut();
        let s: &mut String = &mut guard;
        match value.mutf8_chars(env)?.to_str() {
            std::borrow::Cow::Borrowed(value) => {
                s.clear();
                s.push_str(value);
            }
            std::borrow::Cow::Owned(value) => {
                *s = value;
            }
        }
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[jni_mangle("example.Mangle")]
pub fn drop<'local>(mut env: EnvUnowned<'local>, _class: JClass<'local>, ptr: JOwned<Box<String>>) {
    env.with_env(|_env| -> Result<_, Error> {
        // Fully-qualified to avoid recursing into this `drop` fn.
        std::mem::drop(ptr);
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}
