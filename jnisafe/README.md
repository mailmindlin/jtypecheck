# jnisafe

Runtime wrapper types for safely handing Rust smart pointers to Java across a JNI boundary as opaque `jlong` handles, and recovering them later.

Instead of passing a raw `long` and transmuting it back by hand, you use a typed wrapper that encodes the ownership discipline:

- **`JRef<'a, T>`** — a borrowed, non-null shared pointer (`&T`).
- **`JMut<'a, T>`** — a borrowed, non-null mutable pointer (`&mut T`, `Box` only).
- **`JOwned<T>`** — an owned pointer, used as a return type or a consuming
  parameter (e.g. a `drop` method that frees the object).

Each is `#[repr(transparent)]` over a `jlong`, so they cost nothing at the FFI
boundary, and the null niche lets `Option<JRef<..>>` model a nullable handle
without a wider type.

```rust
use jnisafe::{JOwned, JRef};

// return type: hand ownership of a Box<String> to Java
fn create(value: String) -> JOwned<Box<String>> {
    Box::new(value).into()
}

// parameter: borrow it back immutably
fn read(ptr: JRef<'_, Box<String>>) -> usize {
    ptr.len()
}
```

### With the jni 0.22.4 macros

These wrappers work with the jni crate's macros, too. A `#[jni_mangle]` function
keeps its real signature, so all three wrappers are used as-is. For
`native_method! { … }` / `bind_java_type! { … }`, carry an **owned** handle
through with a `type_map` entry — `#[repr(transparent)]` over `jlong` is exactly
what the `unsafe … => long` mapping needs:

```rust
native_method! {
    java_type = "example.Foo",
    type_map = { unsafe JOwned<Box<String>> => long },
    static fn create(value: JString) -> JOwned<Box<String>>,
}
```

The borrowed `JRef` / `JMut` carry a `'local` lifetime that those macros'
`const`-evaluated context can't name, so reach for `#[jni_mangle]` when a native
takes a borrow handle.

## Runtime validation (debug builds)

The static checker only sees signatures; it can't know *which* handle Java
passes at runtime. In **debug builds** (`debug_assertions`), `jnisafe` keeps a
side table of every live handle and validates it on use, catching bugs the
checker can't:

- a **wrong-type** handle (Java passed a `Box<Foo>` where a `Box<Bar>` was expected),
- a **use-after-free** / **double-free** (a handle used or dropped after it was freed),
- a **bogus or misaligned** `long` that never came from a `jnisafe` handle.

On a violation it prints a diagnostic and panics; inside a JNI call the jni
`with_env` glue surfaces that as a Java `Throwable` with a stack trace. For
**mutable aliasing**, use the checked `borrow_mut()` path:

```rust
let mut guard = ptr.borrow_mut();   // JMut / JOwned
*guard = new_value;                 // two concurrent borrow_mut() of one object panic in debug
```

This is **completely compiled out in release** — the wrappers stay
`#[repr(transparent)]` over a `jlong` with zero added cost. To harden a release
build, opt in per package: `[profile.release.package.jnisafe] debug-assertions = true`.

**Limits:** plain `*ptr = x` via `DerefMut` (and shared-vs-mutable aliasing)
stay unchecked — only `borrow_mut()` guards each other; and a freed address that
the allocator immediately reuses for a *new object of the same type* (ABA) can
slip past the address-keyed table.

Pair this with the [`jnisafe-annotations`] Java annotations and the
[`jnisafe-check`] static checker to verify the Rust and Java sides agree before
they fail at runtime. See the [project README] for the full workflow.

[`jnisafe-annotations`]: https://github.com/mailmindlin/jtypecheck/tree/main/jnisafe-annotations
[`jnisafe-check`]: https://crates.io/crates/jnisafe-check
[project README]: https://github.com/mailmindlin/jtypecheck

## License

Licensed under either of Apache-2.0 or MIT at your option.
