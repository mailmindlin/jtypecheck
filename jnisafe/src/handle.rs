//! The public handle types handed across the JNI boundary: [`JRef`] (borrowed
//! shared), [`JMut`] (borrowed mutable), [`MutGuard`] (a checked exclusive
//! borrow), and [`JOwned`] (owned). Each is layout-identical to a `jlong`.

use std::{
    marker::PhantomData,
    mem::ManuallyDrop,
    num::NonZero,
    ops::{Deref, DerefMut},
    panic::{RefUnwindSafe, UnwindSafe},
    ptr::NonNull,
    sync::Arc,
};

use jni::sys::jlong;

use crate::traits::{
    IntoJavaPtr, IntoJavaPtrMut, expose_as_jlong, from_exposed_jlong, from_exposed_jlong_mut,
};

/// Borrowed, non-null shared pointer to a Java-owned Rust object.
///
/// Receive as a parameter directly (`@Ref(nullable=false)`) or wrapped in
/// `Option` (`@Ref`, nullable): `Option<JRef<T>>` is layout-identical to a bare
/// `jlong` via the null niche, with `0` decoding to `None`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct JRef<'a, T: IntoJavaPtr> {
    internal: NonZero<jlong>,
    lifetime: PhantomData<&'a T>,
}

impl<T: IntoJavaPtr> Deref for JRef<'_, T> {
    type Target = T::Target;

    fn deref(&self) -> &Self::Target {
        rt_validate!(self.internal.get() as usize, T::Target, "JRef::deref");
        let ptr = from_exposed_jlong::<T::Target>(self.internal);
        // Safety: the address came from a non-null pointer whose provenance was
        // exposed when the object was handed to Java, and Java guarantees it
        // stays valid and shared for the duration of this borrow.
        unsafe { &*ptr }
    }
}

impl<T: IntoJavaPtr + Send + Sync> UnwindSafe for JRef<'_, Arc<T>> {}
impl<T: IntoJavaPtr + Send + Sync> RefUnwindSafe for JRef<'_, Arc<T>> {}
impl<T: IntoJavaPtr + Send> UnwindSafe for JRef<'_, Box<T>> {}
impl<T: IntoJavaPtr + Send> RefUnwindSafe for JRef<'_, Box<T>> {}

/// Borrowed, non-null mutable pointer to a Java-owned Rust object.
///
/// `Deref` is available for any [`IntoJavaPtr`]; `DerefMut` only for
/// [`IntoJavaPtrMut`] (i.e. `Box`, not `Arc`). Not `Copy`/`Clone`, since a
/// duplicate mutable handle would alias.
#[repr(transparent)]
#[derive(Debug)]
pub struct JMut<'local, T: IntoJavaPtr> {
    internal: NonZero<jlong>,
    lifetime: PhantomData<&'local mut T>,
}

impl<T: IntoJavaPtr> Deref for JMut<'_, T> {
    type Target = T::Target;
    fn deref(&self) -> &Self::Target {
        rt_validate!(self.internal.get() as usize, T::Target, "JMut::deref");
        let ptr = from_exposed_jlong::<T::Target>(self.internal);
        // Safety: see `JRef::deref`.
        unsafe { &*ptr }
    }
}

impl<T: IntoJavaPtrMut> DerefMut for JMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        rt_validate!(self.internal.get() as usize, T::Target, "JMut::deref_mut");
        let mut ptr = from_exposed_jlong_mut::<T::Target>(self.internal);
        // Safety: `T: IntoJavaPtrMut` guarantees exclusive ownership, and Java's
        // lock discipline guarantees we hold the only mutable handle for the
        // duration of this borrow.
        unsafe { ptr.as_mut() }
    }
}

impl<T: IntoJavaPtrMut> JMut<'_, T> {
    /// Borrow the pointee mutably through a checked guard. In debug builds, a
    /// second concurrent `borrow_mut()` over the *same* object (e.g. Java passed
    /// the same handle to two `@Mut` parameters) panics instead of forming two
    /// aliasing `&mut`.
    ///
    /// This is an opt-in, checked alternative to plain [`DerefMut`]; `*m = x`
    /// via `DerefMut` is left unchanged and stays unchecked for aliasing.
    pub fn borrow_mut(&mut self) -> MutGuard<'_, T> {
        rt_begin_guard!(self.internal.get() as usize, T::Target);
        let ptr = from_exposed_jlong_mut::<T::Target>(self.internal);
        MutGuard {
            ptr,
            _life: PhantomData,
        }
    }
}

/// A checked, exclusive mutable borrow of a Java-owned object, returned by
/// [`JMut::borrow_mut`] / [`JOwned::borrow_mut`]. Dereferences to the pointee
/// like a `&mut T::Target`.
///
/// In debug builds, the guard registers an exclusive-borrow flag on the target
/// object for its lifetime; a second concurrent guard over the same object
/// panics (mutable aliasing). The flag is keyed by the object's address and
/// cleared when the guard drops, so moving the originating handle is harmless.
/// In release builds it is a zero-cost wrapper around the pointer.
pub struct MutGuard<'guard, T: IntoJavaPtrMut> {
    ptr: NonNull<T::Target>,
    _life: PhantomData<&'guard mut T>,
}

impl<T: IntoJavaPtrMut> Deref for MutGuard<'_, T> {
    type Target = T::Target;
    fn deref(&self) -> &Self::Target {
        // Safety: built from a validated, exclusively-borrowed pointer that
        // outlives the guard (`'a`).
        unsafe { self.ptr.as_ref() }
    }
}

impl<T: IntoJavaPtrMut> DerefMut for MutGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Safety: see `Deref`; the guard holds the unique mutable borrow.
        unsafe { self.ptr.as_mut() }
    }
}

impl<T: IntoJavaPtrMut> Drop for MutGuard<'_, T> {
    fn drop(&mut self) {
        rt_end_guard!(self.ptr.as_ptr() as usize);
    }
}

/// Owned pointer to a Java-stored Rust object.
///
/// Used as a native-method return type (`@Owned`) and as a by-value parameter
/// that consumes the object (e.g. a `drop` method). Internally nullable: the
/// error path of `EnvOutcome::resolve` must return `T::default()` after
/// throwing, so an owned return type has to be able to represent null. A
/// null `JOwned` exists only transiently on that error path and is never
/// dereferenced.
#[repr(transparent)]
#[derive(Debug)]
pub struct JOwned<T: IntoJavaPtr> {
    internal: Option<NonZero<jlong>>,
    ty: PhantomData<T>,
}

impl<T: IntoJavaPtr> JOwned<T> {
    /// Construct a null `JOwned<T>`
    #[must_use]
    pub const fn null() -> Self {
        Self {
            internal: None,
            ty: PhantomData,
        }
    }

    /// Recover the owned smart pointer, or `None` if this `JOwned` is null.
    #[must_use]
    pub fn into_inner(self) -> Option<T> {
        let this = ManuallyDrop::new(self);
        let internal = this.internal?;
        rt_validate!(internal.get() as usize, T::Target, "JOwned::into_inner");
        rt_deregister!(internal.get() as usize, "JOwned::into_inner");
        let mut ptr = from_exposed_jlong_mut::<T::Target>(internal);
        // Safety: a non-null `internal` was produced by `From`, which stored
        // a pointer from `IntoJavaPtr::to_raw`; we reconstruct exactly once
        // and suppress `Drop` via `ManuallyDrop`.
        Some(unsafe { T::from_raw(ptr.as_mut()) })
    }

    /// Get a reference to the contained value
    pub fn get(&self) -> Option<&T::Target> {
        let internal = self.internal?;
        rt_validate!(internal.get() as usize, T::Target, "JOwned::get");
        let ptr = from_exposed_jlong::<T::Target>(internal);
        unsafe { ptr.as_ref() }
    }

    /// Get a mutable reference to the contained value
    pub fn get_mut(&mut self) -> Option<&mut T::Target> {
        let internal = self.internal?;
        rt_validate!(internal.get() as usize, T::Target, "JOwned::get_mut");
        let mut ptr = from_exposed_jlong_mut::<T::Target>(internal);
        Some(unsafe { ptr.as_mut() })
    }

    /// Borrow the pointee mutably through a checked guard. In debug builds, a
    /// second concurrent `borrow_mut()` over the *same* object (e.g. Java passed
    /// the same handle to two `@Mut` parameters) panics instead of forming two
    /// aliasing `&mut`. Panics if this `JOwned` is null. Requires `T:
    /// IntoJavaPtrMut` (`Box`, not `Arc`).
    pub fn borrow_mut(&mut self) -> Option<MutGuard<'_, T>>
    where
        T: IntoJavaPtrMut,
    {
        let internal = self.internal?;
        rt_begin_guard!(internal.get() as usize, T::Target);
        let ptr = from_exposed_jlong_mut::<T::Target>(internal);
        Some(MutGuard {
            ptr,
            _life: PhantomData,
        })
    }
}

/// Construct a null `JOwned<T>`
impl<T: IntoJavaPtr> Default for JOwned<T> {
    fn default() -> Self {
        Self::null()
    }
}

impl<R: IntoJavaPtr> From<R> for JOwned<R> {
    fn from(value: R) -> Self {
        // Safety: we take ownership of `value` and hand the raw pointer to Java;
        // ownership is recovered exactly once via `take`/`Drop`.
        let raw = unsafe { value.to_raw() };
        let internal = expose_as_jlong(raw);
        rt_register!(internal.get() as usize, R::Target);
        Self {
            internal: Some(internal),
            ty: PhantomData,
        }
    }
}

impl<T: IntoJavaPtr> Deref for JOwned<T> {
    type Target = T::Target;
    fn deref(&self) -> &Self::Target {
        let internal = self.internal.expect("deref of null JOwned");
        rt_validate!(internal.get() as usize, T::Target, "JOwned::deref");
        let ptr = from_exposed_jlong::<T::Target>(internal);
        // Safety: non-null `internal` points at a live object owned by `self`.
        unsafe { &*ptr }
    }
}

impl<T: IntoJavaPtrMut> DerefMut for JOwned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let internal = self.internal.expect("deref of null JOwned");
        rt_validate!(internal.get() as usize, T::Target, "JOwned::deref_mut");
        let mut ptr = from_exposed_jlong_mut::<T::Target>(internal);
        // Safety: `T: IntoJavaPtrMut` guarantees exclusive ownership and `self`
        // owns the live object.
        unsafe { ptr.as_mut() }
    }
}

impl<T: IntoJavaPtr> Drop for JOwned<T> {
    fn drop(&mut self) {
        if let Some(internal) = self.internal {
            rt_validate!(internal.get() as usize, T::Target, "JOwned::drop");
            rt_deregister!(internal.get() as usize, "JOwned::drop");
            let ptr = from_exposed_jlong_mut::<T::Target>(internal);
            // Safety: non-null `internal` was produced by `From` and is dropped
            // exactly once (here, unless previously consumed by `take`, which
            // forgets `self`).
            drop(unsafe { T::from_raw(ptr.as_ptr()) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn owned_round_trips_value() {
        let owned: JOwned<Box<String>> = Box::new("hello".to_string()).into();
        assert!(owned.internal.is_some(), "non-null after From");
        assert_eq!(&**owned, "hello");
        let recovered = owned.into_inner().expect("non-null take yields Some");
        assert_eq!(*recovered, "hello");
    }

    #[test]
    fn default_is_null() {
        let owned: JOwned<Box<String>> = JOwned::default();
        assert_eq!(owned.internal, None);
        assert!(owned.into_inner().is_none(), "null take yields None");
    }

    struct DropCounter(&'static AtomicUsize);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    static DROPS: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn drop_frees_exactly_once() {
        DROPS.store(0, Ordering::SeqCst);
        {
            let _owned: JOwned<Box<DropCounter>> = Box::new(DropCounter(&DROPS)).into();
            assert_eq!(DROPS.load(Ordering::SeqCst), 0, "not dropped while held");
        }
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            1,
            "dropped once when JOwned drops"
        );
    }

    #[test]
    fn take_suppresses_drop() {
        DROPS.store(0, Ordering::SeqCst);
        let owned: JOwned<Box<DropCounter>> = Box::new(DropCounter(&DROPS)).into();
        let recovered = owned.into_inner().expect("Some");
        assert_eq!(DROPS.load(Ordering::SeqCst), 0, "take must not drop");
        drop(recovered);
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            1,
            "dropped once via recovered value"
        );
    }

    #[test]
    fn layout_matches_jlong() {
        assert_eq!(size_of::<JOwned<Box<String>>>(), size_of::<jlong>());
        assert_eq!(align_of::<JOwned<Box<String>>>(), align_of::<jlong>());
        // Null niche: Option<JRef> is the same size as a bare jlong.
        assert_eq!(
            size_of::<Option<JRef<'static, Box<String>>>>(),
            size_of::<jlong>()
        );
        assert_eq!(size_of::<JRef<'static, Box<String>>>(), size_of::<jlong>());
        assert_eq!(size_of::<JMut<'static, Box<String>>>(), size_of::<jlong>());
    }

    // --- Debug-only runtime-validation tests ------------------------------
    // These run under `cargo test` (dev profile → debug_assertions on). Handles
    // are fabricated via the crate-private wrapper fields to drive the violation
    // paths; backing memory is kept valid so the check (not a real UB read) is
    // what fires.

    #[cfg(debug_assertions)]
    #[test]
    fn registry_round_trip_is_clean() {
        let owned: JOwned<Box<String>> = Box::new("hi".to_string()).into();
        assert_eq!(&**owned, "hi"); // deref validates
        assert_eq!(owned.into_inner().as_deref().map(String::as_str), Some("hi"));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "type mismatch")]
    fn wrong_type_handle_detected() {
        let owned: JOwned<Box<String>> = Box::new("hi".to_string()).into();
        // A borrow claiming the wrong pointee type over a live address.
        let wrong: JRef<'static, Box<u64>> = JRef {
            internal: owned.internal.unwrap(),
            lifetime: PhantomData,
        };
        let _ = *wrong; // validate → type mismatch (before any real deref)
        drop(owned);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "no live handle")]
    fn use_after_free_detected() {
        let owned: JOwned<Box<String>> = Box::new("hi".to_string()).into();
        let addr = owned.internal.unwrap();
        // `take` deregisters but keeps the allocation alive in `_recovered`.
        let _recovered = owned.into_inner().unwrap();
        let stale: JRef<'static, Box<String>> = JRef {
            internal: addr,
            lifetime: PhantomData,
        };
        let _ = &*stale; // validate → no live handle
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "no live handle")]
    fn double_free_detected() {
        let real: JOwned<Box<String>> = Box::new("x".to_string()).into();
        // A duplicate owned handle over the same registration.
        let dup: JOwned<Box<String>> = JOwned {
            internal: real.internal,
            ty: PhantomData,
        };
        let _ = real.into_inner(); // deregisters + frees the allocation
        drop(dup); // JOwned::drop validates → no live handle (before the real free)
    }

    #[cfg(debug_assertions)]
    #[test]
    fn arc_owner_refcount() {
        let arc = Arc::new(7u64);
        let o1: JOwned<Arc<u64>> = arc.clone().into();
        let o2: JOwned<Arc<u64>> = arc.clone().into();
        assert_eq!(o1.internal, o2.internal, "Arc clones share one address");
        let _ = o1.into_inner(); // owners 2 -> 1; address must stay live
        // Would panic with "no live handle" if the refcount had hit zero early.
        assert_eq!(o2.get().copied(), Some(7));
        drop(o2); // owners 1 -> 0, removed
        drop(arc);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "aliasing")]
    fn borrow_mut_concurrent_guards_detected() {
        let owned: JOwned<Box<u64>> = Box::new(1u64).into();
        let addr = owned.internal.unwrap();
        // Two distinct JMut over one object — Java double-passing a @Mut handle.
        let mut m1: JMut<'static, Box<u64>> = JMut {
            internal: addr,
            lifetime: PhantomData,
        };
        let mut m2: JMut<'static, Box<u64>> = JMut {
            internal: addr,
            lifetime: PhantomData,
        };
        let _g1 = m1.borrow_mut();
        let _g2 = m2.borrow_mut(); // already mutably borrowed → aliasing
        drop(owned);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn borrow_mut_sequential_is_clean() {
        let owned: JOwned<Box<u64>> = Box::new(1u64).into();
        let addr = owned.internal.unwrap();
        let mut m1: JMut<'static, Box<u64>> = JMut {
            internal: addr,
            lifetime: PhantomData,
        };
        let mut m2: JMut<'static, Box<u64>> = JMut {
            internal: addr,
            lifetime: PhantomData,
        };
        {
            let mut g = m1.borrow_mut();
            *g = 5;
        } // guard dropped → flag cleared
        {
            let mut g = m2.borrow_mut(); // sequential, not aliasing → fine
            *g += 1;
        }
        assert_eq!(owned.get().copied(), Some(6));
        drop(owned);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn release_checks_compiled_out() {
        // Runs under `cargo test --release`: no registry, no instrumentation —
        // a plain round-trip still works, documenting the zero-cost path.
        let owned: JOwned<Box<String>> = Box::new("hi".to_string()).into();
        assert_eq!(&**owned, "hi");
        assert_eq!(
            owned.into_inner().as_deref().map(String::as_str),
            Some("hi")
        );
    }
}
