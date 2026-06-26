//! `jnisafe` — runtime wrapper types for handing Rust smart pointers across the
//! JNI boundary as opaque `jlong` handles and recovering them safely.
//!
//! The public surface is the handle types in [`handle`] ([`JOwned`], [`JRef`],
//! [`JMut`], [`MutGuard`]) built on the [`IntoJavaPtr`] / [`IntoJavaPtrMut`]
//! traits in [`traits`]. In debug builds every handle is validated against a
//! side table (see the `registry` module) to catch wrong-type / freed /
//! double-freed / bogus handles; in release the checks compile out and the
//! wrappers are byte-for-byte a plain `jlong`.
#![warn(clippy::pedantic)]
#![forbid(clippy::missing_safety_doc)]

// --- Debug-only runtime handle validation ---------------------------------
//
// In debug builds (`debug_assertions`) the handle methods record and validate
// every handle against a side table (see `mod registry`), catching wrong-type /
// freed / double-freed / bogus handles that Java can hand back. Each macro
// expands to nothing in release, so the wrappers stay zero-cost and byte-for-byte
// identical to a plain `jlong` transmute. The wrapper structs are never widened —
// all state lives in the side table.
//
// These `macro_rules!` are defined here, ahead of the `mod` declarations below,
// so the `traits`/`handle` modules see them in textual scope without `#[macro_use]`.

/// Register a freshly-created owned handle (`JOwned::from`).
macro_rules! rt_register {
    ($addr:expr, $ty:ty) => {{
        #[cfg(debug_assertions)]
        $crate::registry::register(
            $addr,
            ::std::any::TypeId::of::<$ty>(),
            ::std::any::type_name::<$ty>(),
        );
    }};
}

/// Validate that a handle is live and of the expected type before use.
macro_rules! rt_validate {
    ($addr:expr, $ty:ty, $op:expr) => {{
        #[cfg(debug_assertions)]
        $crate::registry::validate(
            $addr,
            ::std::any::TypeId::of::<$ty>(),
            ::std::any::type_name::<$ty>(),
            $op,
        );
    }};
}

/// Drop a live handle's registration (`take` / `Drop`); double-free detector.
macro_rules! rt_deregister {
    ($addr:expr, $op:expr) => {{
        #[cfg(debug_assertions)]
        $crate::registry::deregister($addr, $op);
    }};
}

/// Take the exclusive mutable-borrow flag for a `borrow_mut()` guard.
macro_rules! rt_begin_guard {
    ($addr:expr, $ty:ty) => {{
        #[cfg(debug_assertions)]
        $crate::registry::begin_mut_guard(
            $addr,
            ::std::any::TypeId::of::<$ty>(),
            ::std::any::type_name::<$ty>(),
        );
    }};
}

/// Release a `borrow_mut()` guard's exclusive-borrow flag (guard `Drop`).
macro_rules! rt_end_guard {
    ($addr:expr) => {{
        #[cfg(debug_assertions)]
        $crate::registry::end_mut_guard($addr);
    }};
}

/// Cheap alignment check when reconstructing a pointer from a `jlong`.
macro_rules! rt_check_align {
    ($addr:expr, $ty:ty) => {{
        #[cfg(debug_assertions)]
        $crate::registry::check_align::<$ty>($addr);
    }};
}

mod handle;
mod traits;

#[cfg(debug_assertions)]
mod registry;

pub use handle::{JMut, JOwned, JRef, MutGuard};
pub use traits::{IntoJavaPtr, IntoJavaPtrMut};
