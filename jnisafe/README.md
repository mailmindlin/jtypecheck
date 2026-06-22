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

Pair this with the [`jnisafe-annotations`] Java annotations and the
[`jnisafe-check`] static checker to verify the Rust and Java sides agree before
they fail at runtime. See the [project README] for the full workflow.

[`jnisafe-annotations`]: https://github.com/mailmindlin/jtypecheck/tree/main/jnisafe-annotations
[`jnisafe-check`]: https://crates.io/crates/jnisafe-check
[project README]: https://github.com/mailmindlin/jtypecheck

## License

Licensed under either of Apache-2.0 or MIT at your option.
