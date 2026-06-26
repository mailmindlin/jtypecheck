# Example

A small, complete, **correct** JNI layer you can read end to end: a Java API
under [`java/example`](java/example) and its compiling Rust implementation under
[`rust`](rust/src). Each class demonstrates one way to declare natives so the
[`jnisafe-check`](../jnisafe-check) checker can verify the two sides agree.

| Java class                                         | Rust                                                | Demonstrates                                             |
| -------------------------------------------------- | --------------------------------------------------- | -------------------------------------------------------- |
| [`HandWritten`](java/example/HandWritten.java)     | [`hand_written.rs`](rust/src/hand_written.rs)       | Hand-written `Java_*` exports with `JOwned`/`JRef`/`JMut` |
| [`Mangle`](java/example/Mangle.java)               | [`mangle.rs`](rust/src/mangle.rs)                   | `#[jni_mangle]` — the **same** contract, incl. borrows   |
| [`NativeMethod`](java/example/NativeMethod.java)   | [`native_method.rs`](rust/src/native_method.rs)     | `native_method!` with an owned-handle `type_map`         |
| [`BindType`](java/example/BindType.java)           | [`bind_type.rs`](rust/src/bind_type.rs)             | `bind_java_type!` — natives **and** Rust→Java calls      |
| [`Overloaded`](java/example/Overloaded.java)       | [`overloaded.rs`](rust/src/overloaded.rs)           | Overloaded natives resolving to the long mangled form    |

**Start with `HandWritten` and `Mangle`.** They implement the *identical*
create/tryGet/get/set/drop contract — diffing
[`hand_written.rs`](rust/src/hand_written.rs) against
[`mangle.rs`](rust/src/mangle.rs) shows exactly what `#[jni_mangle]` changes
(only the export boilerplate) and nothing else. The remaining files differ only
where a binding style's capabilities force them to: `native_method!` can't name
the lifetime of borrowed handles, and `bind_java_type!` / `Overloaded` show
features the others have no equivalent for.

Run the checker against it (from the workspace root):

```bash
pixi run demo
```

The full walkthrough of each binding style is in the
[top-level README](../README.md#3-implement-the-natives-in-rust-with-jnisafe-wrappers).
The intentionally-broken counterparts used by the test suite live under
[`jnisafe-check/tests/fixtures`](../jnisafe-check/tests/fixtures).
