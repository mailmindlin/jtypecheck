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
        unsafe { Arc::from_raw(ptr as *const T) }
    }

    unsafe fn to_raw(self) -> NonNull<T> {
        let ptr = Arc::into_raw(self) as *mut T;
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
/// (`Box`) implement this, which is what restricts [`DerefMut`] on [`JMut`] /
/// [`JOwned`] to `Box`-backed pointers.
///
/// # Safety
///
/// Implementors must guarantee that, while a value is borrowed by Java as a
/// mutable pointer, no other reference to the pointee exists — i.e. the smart
/// pointer is the unique owner of its allocation.
pub unsafe trait IntoJavaPtrMut: IntoJavaPtr {}

unsafe impl<T: Send + 'static> IntoJavaPtrMut for Box<T> {}

/// Assert at compile time that a `usize` address fits in a `jlong`.
const _: () = assert!(size_of::<jlong>() >= size_of::<usize>());

/// Convert a raw pointer into a `jlong`, *exposing* its provenance so the
/// address can later be turned back into a usable pointer with
/// [`with_exposed_provenance`](std::ptr::with_exposed_provenance).
fn expose_as_jlong<T>(ptr: NonNull<T>) -> NonZero<jlong> {
    let raw = ptr.as_ptr().expose_provenance() as jlong;
    NonZero::new(raw).unwrap()
}

/// Reconstruct a shared pointer from an exposed `jlong` address.
fn from_exposed_jlong<T>(addr: NonZero<jlong>) -> *const T {
    std::ptr::with_exposed_provenance::<T>(addr.get() as usize)
}

/// Reconstruct a mutable pointer from an exposed `jlong` address.
fn from_exposed_jlong_mut<T>(addr: NonZero<jlong>) -> NonNull<T> {
    let addr: NonZero<usize> = addr.try_into().unwrap();

    NonNull::with_exposed_provenance(addr)
}

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

impl<'a, T: IntoJavaPtr> Deref for JRef<'a, T> {
    type Target = T::Target;

    fn deref(&self) -> &Self::Target {
        let ptr = from_exposed_jlong::<T::Target>(self.internal);
        // Safety: the address came from a non-null pointer whose provenance was
        // exposed when the object was handed to Java, and Java guarantees it
        // stays valid and shared for the duration of this borrow.
        unsafe { &*ptr }
    }
}

impl<'a, T: IntoJavaPtr + Send + Sync> UnwindSafe for JRef<'a, Arc<T>> {}
impl<'a, T: IntoJavaPtr + Send + Sync> RefUnwindSafe for JRef<'a, Arc<T>> {}
impl<'a, T: IntoJavaPtr + Send> UnwindSafe for JRef<'a, Box<T>> {}
impl<'a, T: IntoJavaPtr + Send> RefUnwindSafe for JRef<'a, Box<T>> {}

/// Borrowed, non-null mutable pointer to a Java-owned Rust object.
///
/// `Deref` is available for any [`IntoJavaPtr`]; `DerefMut` only for
/// [`IntoJavaPtrMut`] (i.e. `Box`, not `Arc`). Not `Copy`/`Clone`, since a
/// duplicate mutable handle would alias.
#[repr(transparent)]
#[derive(Debug)]
pub struct JMut<'a, T: IntoJavaPtr> {
    internal: NonZero<jlong>,
    lifetime: PhantomData<&'a mut T>,
}

impl<'a, T: IntoJavaPtr> Deref for JMut<'a, T> {
    type Target = T::Target;
    fn deref(&self) -> &Self::Target {
        let ptr = from_exposed_jlong::<T::Target>(self.internal);
        // Safety: see `JRef::deref`.
        unsafe { &*ptr }
    }
}

impl<'a, T: IntoJavaPtrMut> DerefMut for JMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let mut ptr = from_exposed_jlong_mut::<T::Target>(self.internal);
        // Safety: `T: IntoJavaPtrMut` guarantees exclusive ownership, and Java's
        // lock discipline guarantees we hold the only mutable handle for the
        // duration of this borrow.
        unsafe { ptr.as_mut() }
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
    pub const fn null() -> Self {
        Self {
            internal: None,
            ty: PhantomData,
        }
    }

    /// Recover the owned smart pointer, or `None` if this `JOwned` is null.
    pub fn take(self) -> Option<T> {
        let this = ManuallyDrop::new(self);
        let mut ptr = from_exposed_jlong_mut::<T::Target>(this.internal?);
        // Safety: a non-null `internal` was produced by `From`, which stored
        // a pointer from `IntoJavaPtr::to_raw`; we reconstruct exactly once
        // and suppress `Drop` via `ManuallyDrop`.
        Some(unsafe { T::from_raw(ptr.as_mut()) })
    }

    /// Get a reference to the contained value
    pub fn get(&self) -> Option<&T::Target> {
        let ptr = from_exposed_jlong::<T::Target>(self.internal?);
        unsafe { ptr.as_ref() }
    }

    /// Get a mutable reference to the contained value
    pub fn get_mut(&mut self) -> Option<&mut T::Target> {
        let mut ptr = from_exposed_jlong_mut::<T::Target>(self.internal?);
        Some(unsafe { ptr.as_mut() })
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
        Self {
            internal: Some(expose_as_jlong(raw)),
            ty: PhantomData,
        }
    }
}

impl<T: IntoJavaPtr> Deref for JOwned<T> {
    type Target = T::Target;
    fn deref(&self) -> &Self::Target {
        let internal = self.internal.expect("deref of null JOwned");
        let ptr = from_exposed_jlong::<T::Target>(internal);
        // Safety: non-null `internal` points at a live object owned by `self`.
        unsafe { &*ptr }
    }
}

impl<T: IntoJavaPtrMut> DerefMut for JOwned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let internal = self.internal.expect("deref of null JOwned");
        let mut ptr = from_exposed_jlong_mut::<T::Target>(internal);
        // Safety: `T: IntoJavaPtrMut` guarantees exclusive ownership and `self`
        // owns the live object.
        unsafe { ptr.as_mut() }
    }
}

impl<T: IntoJavaPtr> Drop for JOwned<T> {
    fn drop(&mut self) {
        if let Some(internal) = self.internal {
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
        let recovered = owned.take().expect("non-null take yields Some");
        assert_eq!(*recovered, "hello");
    }

    #[test]
    fn default_is_null() {
        let owned: JOwned<Box<String>> = JOwned::default();
        assert_eq!(owned.internal, None);
        assert!(owned.take().is_none(), "null take yields None");
    }

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
        let recovered = owned.take().expect("Some");
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
}
