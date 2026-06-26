//! Debug-only side table that records every live handle (by pointer address)
//! and validates handles on use. Compiled out entirely in release builds.
//!
//! On a detected violation we `eprintln!` a diagnostic and `panic!`; the panic
//! is raised *before* any unsafe dereference, so it never executes UB. Inside a
//! JNI call the jni `with_env` glue converts the panic into a Java `Throwable`.

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

struct Record {
    type_id: TypeId,
    type_name: &'static str,
    /// Live owned handles at this address (`Arc` may share one address).
    owners: u32,
    /// Set while a `borrow_mut()` guard holds the exclusive borrow.
    mut_guarded: bool,
}

fn table() -> &'static Mutex<HashMap<usize, Record>> {
    static TABLE: OnceLock<Mutex<HashMap<usize, Record>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock() -> MutexGuard<'static, HashMap<usize, Record>> {
    // Tolerate poisoning: a prior validation panic must not wedge the table.
    table().lock().unwrap_or_else(PoisonError::into_inner)
}

/// Report a violation and stop. Callers release the table lock first, so the
/// panic neither poisons it for other threads nor runs over a bad pointer.
fn report(addr: usize, op: &str, msg: std::fmt::Arguments) -> ! {
    eprintln!(
        "\n=== jnisafe runtime-check violation ===\noperation: {op}\nhandle: {addr:#x}\n{msg}\n"
    );
    panic!("jnisafe runtime-check violation in {op} @ {addr:#x}: {msg}");
}

/// Record a freshly-created owned handle. For `Arc`, a repeated address with
/// a matching type bumps the owner count.
pub fn register(addr: usize, type_id: TypeId, type_name: &'static str) {
    let mut t = lock();
    if let Some(rec) = t.get_mut(&addr) {
        if rec.type_id != type_id {
            let had = rec.type_name;
            drop(t);
            report(
                addr,
                "JOwned::from",
                format_args!(
                    "address re-registered with a different type: had `{had}`, now `{type_name}`"
                ),
            );
        }
        rec.owners += 1;
    } else {
        t.insert(
            addr,
            Record {
                type_id,
                type_name,
                owners: 1,
                mut_guarded: false,
            },
        );
    }
}

/// Validate a handle is live and of the expected type before it is used.
pub fn validate(addr: usize, expected: TypeId, expected_name: &'static str, op: &'static str) {
    let t = lock();
    match t.get(&addr) {
        None => {
            drop(t);
            report(
                addr,
                op,
                format_args!(
                    "no live handle registered here — use-after-free, double-free, or a bogus `long` that did not come from a jnisafe handle (expected `{expected_name}`)"
                ),
            );
        }
        Some(rec) if rec.type_id != expected => {
            let had = rec.type_name;
            drop(t);
            report(
                addr,
                op,
                format_args!(
                    "handle type mismatch: expected `{expected_name}`, but this handle holds `{had}`"
                ),
            );
        }
        Some(_) => {}
    }
}

/// Drop a live handle's registration; an absent address is a double-free.
pub fn deregister(addr: usize, op: &'static str) {
    let mut t = lock();
    match t.get_mut(&addr) {
        None => {
            drop(t);
            report(
                addr,
                op,
                format_args!(
                    "freeing a handle that is not live — double-free, or it was never created via jnisafe"
                ),
            );
        }
        Some(rec) => {
            rec.owners -= 1;
            if rec.owners == 0 {
                t.remove(&addr);
            }
        }
    }
}

/// Take the exclusive mutable-borrow flag for a `borrow_mut()` guard. Panics
/// if the object is already mutably borrowed via another live guard.
pub fn begin_mut_guard(addr: usize, expected: TypeId, expected_name: &'static str) {
    validate(addr, expected, expected_name, "borrow_mut");
    let mut t = lock();
    // `validate` above guarantees the record exists.
    let Some(rec) = t.get_mut(&addr) else { return };
    if rec.mut_guarded {
        drop(t);
        report(
            addr,
            "borrow_mut",
            format_args!(
                "mutable aliasing: this object is already mutably borrowed via another handle's borrow_mut() guard"
            ),
        );
    }
    rec.mut_guarded = true;
}

/// Release a `borrow_mut()` guard's exclusive-borrow flag.
pub fn end_mut_guard(addr: usize) {
    if let Some(rec) = lock().get_mut(&addr) {
        rec.mut_guarded = false;
    }
}

/// Cheap alignment sanity check on a reconstructed handle address.
pub fn check_align<T>(addr: usize) {
    let align = align_of::<T>();
    if align > 1 && !addr.is_multiple_of(align) {
        report(
            addr,
            "from_exposed_jlong",
            format_args!(
                "misaligned handle: address is not aligned to {align} bytes for `{}`",
                std::any::type_name::<T>()
            ),
        );
    }
}
