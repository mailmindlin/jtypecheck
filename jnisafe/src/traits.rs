//! The pointer-conversion foundation: the [`IntoJavaPtr`] / [`IntoJavaPtrMut`]
//! traits that describe which smart pointers can be handed to Java as an opaque
//! `jlong`, plus the `jlong`↔pointer helpers the handle types build on.

use std::{num::NonZero, ops::Deref, ptr::NonNull, sync::Arc};

use jni::sys::jlong;

/// A Rust smart pointer that can be handed to Java as an opaque `jlong` and
/// later recovered.
///
/// # Safety
///
/// Implementors must guarantee that [`to_raw`](IntoJavaPtr::to_raw) and
/// [`from_raw`](IntoJavaPtr::from_raw) round-trip: a pointer produced by
/// `to_raw` may be passed back to `from_raw` exactly once to reconstitute an
/// equivalent value. `Target` must be the type the raw pointer actually points
/// at, so that `&Target` is a valid shared reference for the lifetime the
/// pointer is borrowed by Java.
pub unsafe trait IntoJavaPtr: 'static + Deref<Target: Sized + 'static> {
    /// # Safety
    ///
    /// `ptr` must have been produced by a prior [`to_raw`](Self::to_raw) call on
    /// this same type and not yet reclaimed; it is consumed exactly once.
    unsafe fn from_raw(ptr: *mut Self::Target) -> Self;

    /// # Safety
    ///
    /// The returned pointer transfers ownership out of `self`; it must be handed
    /// back to [`from_raw`](Self::from_raw) exactly once to avoid a leak or
    /// double free.
    unsafe fn to_raw(self) -> NonNull<Self::Target>;
}

unsafe impl<T: Send + Sync + 'static> IntoJavaPtr for Arc<T> {
    unsafe fn from_raw(ptr: *mut T) -> Self {
        unsafe { Arc::from_raw(ptr.cast_const()) }
    }

    unsafe fn to_raw(self) -> NonNull<T> {
        let ptr = Arc::into_raw(self).cast_mut();
        NonNull::new(ptr).unwrap()
    }
}

unsafe impl<T: Send + 'static> IntoJavaPtr for Box<T> {
    unsafe fn from_raw(ptr: *mut T) -> Self {
        unsafe { Box::from_raw(ptr) }
    }

    unsafe fn to_raw(self) -> NonNull<T> {
        //TODO: use Box::into_non_null when it stabilizes
        NonNull::new(Box::into_raw(self)).unwrap()
    }
}

/// Marker for exclusively-owned smart pointers that may safely hand out `&mut T`.
///
/// `Arc` is intentionally excluded: other clones may alias the pointee, so a
/// `&mut T` through an `Arc` would be unsound. Only single-owner pointers
/// (`Box`) implement this, which is what restricts [`DerefMut`](std::ops::DerefMut)
/// on [`JMut`](crate::JMut) / [`JOwned`](crate::JOwned) to `Box`-backed pointers.
///
/// # Safety
///
/// Implementors must guarantee that, while a value is borrowed by Java as a
/// mutable pointer, no other reference to the pointee exists — i.e. the smart
/// pointer is the unique owner of its allocation.
pub unsafe trait IntoJavaPtrMut: IntoJavaPtr {}

unsafe impl<T: Send + 'static> IntoJavaPtrMut for Box<T> {}

/// Assert at compile time that a `usize` address fits in a `jlong`.
const _: () = assert!(
    size_of::<jlong>() >= size_of::<usize>(),
    "target pointer must fit within jlong"
);

/// Convert a raw pointer into a `jlong`, *exposing* its provenance so the
/// address can later be turned back into a usable pointer with
/// [`with_exposed_provenance`](std::ptr::with_exposed_provenance).
pub(crate) fn expose_as_jlong<T>(ptr: NonNull<T>) -> NonZero<jlong> {
    let raw = ptr.as_ptr().expose_provenance().cast_signed() as jlong;
    NonZero::new(raw).unwrap()
}

/// Recover the machine address from an exposed `jlong` handle.
///
/// [`expose_as_jlong`] widened a `usize` address into the 64-bit `jlong`. On
/// 64-bit targets `usize` is also 64-bit, so this round-trip is lossless. On
/// 32-bit targets a `jlong` whose high bits are set could never have been a
/// pointer we produced, so `try_into` rejects it rather than silently
/// truncating to a bogus 32-bit address.
pub(crate) fn jlong_to_addr(handle: NonZero<jlong>) -> NonZero<usize> {
    handle
        .cast_unsigned()
        .try_into()
        .expect("jlong handle address does not fit in a pointer")
}

/// Reconstruct a shared pointer from an exposed `jlong` address.
pub(crate) fn from_exposed_jlong<T>(handle: NonZero<jlong>) -> *const T {
    let addr = jlong_to_addr(handle);
    rt_check_align!(addr.get(), T);
    std::ptr::with_exposed_provenance::<T>(addr.get())
}

/// Reconstruct a mutable pointer from an exposed `jlong` address.
pub(crate) fn from_exposed_jlong_mut<T>(handle: NonZero<jlong>) -> NonNull<T> {
    let addr = jlong_to_addr(handle);
    rt_check_align!(addr.get(), T);
    NonNull::with_exposed_provenance(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_address_round_trips_via_exposed_provenance() {
        // Box -> raw -> jlong -> read back the original value.
        let value = Box::new(0xABCD_1234u64);
        let raw = Box::into_raw(value);
        let addr = expose_as_jlong(NonNull::new(raw).unwrap());
        let back = from_exposed_jlong::<u64>(addr);
        assert_eq!(unsafe { *back }, 0xABCD_1234);
        // Reclaim to avoid leaking.
        drop(unsafe { Box::from_raw(from_exposed_jlong_mut::<u64>(addr).as_ptr()) });
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "misaligned")]
    fn misaligned_handle_detected() {
        // `u64` needs 8-byte alignment; an odd address can't be a real handle.
        let bad = NonZero::new(0x1003 as jlong).unwrap();
        let _ = from_exposed_jlong::<u64>(bad);
    }
}
