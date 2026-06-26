# jnisafe

This project helps annotate a Rust/Java JNI layer to statically check that it agrees. It allows you to check:
* Method names
* Method signatures (arity, parameter types)
* Ownership of `long`-as-pointer handles

You annotate the Java side with `@Owned` / `@Ref` / `@Mut` (from [`jnisafe-annotations`](jnisafe-annotations/)), implement the natives in Rust with the [`jnisafe`](jnisafe/) wrapper types, and run [`jnisafe-check`](jnisafe-check/) to verify the two sides match before they fail at runtime.

```java
// Java: Annotate pointer types
private static native @Owned("Box<String>") long create(String value);
private static native String tryGet(@Ref("Box<String>") long ptr);
```
```rust
// Rust: jnisafe-check verifies these line up with the Java declarations above
#[no_mangle]
pub extern "system" fn Java_example_HandWritten_create(/* … */) -> JOwned<Box<String>> { /* … */ }
```

Can you get around it? Sure. But it's better than nothing.

On top of the static check, the [`jnisafe`](jnisafe/) wrapper types add **debug-only
runtime validation**: in debug builds they track every live handle and catch a
wrong-type, freed, double-freed, or bogus `long` the moment Java passes it back
(surfaced as a Java exception), plus a checked `borrow_mut()` for mutable
aliasing. It compiles out entirely in release — see the [`jnisafe` README](jnisafe/README.md).

## Getting started

### 1. Install the checker

```bash
cargo install jnisafe-check
```

### 2. Add the annotations to your Java project

Depend on `jnisafe-annotations` from Maven Central:

```xml
<dependency>
  <groupId>io.github.mailmindlin</groupId>
  <artifactId>jnisafe-annotations</artifactId>
  <version>0.1.0</version>
</dependency>
```

Then annotate the pointer parameters and return values of your `native` methods:

```java
import io.github.mailmindlin.jnisafe.Mut;
import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

public class Counter {
    // create() hands ownership of the pointer to Java
    private static native @Owned("Box<u64>") long create();
    // get() borrows it immutably, set() borrows it mutably
    private static native long get(@Ref("Box<u64>") long ptr);
    private static native void set(@Mut("Box<u64>") long ptr, long value);
    // drop() takes ownership back so Rust can free it
    private static native void drop(@Owned("Box<u64>") long ptr);
}
```

### 3. Implement the natives in Rust with `jnisafe` wrappers

Add `jnisafe` to your crate and use `JOwned` / `JRef` / `JMut` in place of raw `long`s.
See [`example/rust`](example/rust/src/hand_written.rs) for a complete, compiling implementation.

#### Using the jni 0.22.4 ergonomic macros

`jnisafe-check` also understands natives declared with the jni crate's macros, so
you keep the static check whichever style you adopt:

* **`#[jni_mangle("pkg.Class")]`** — the function keeps its real signature, so
  `JOwned` / `JRef` / `JMut` work transparently, exactly as in a hand-written
  `Java_*` export. The Java method name is derived from the Rust fn name
  (`set_value` → `setValue`). See [`mangle.rs`](example/rust/src/mangle.rs).
* **`native_method! { … }`** and **`bind_java_type! { native_methods { … } }`** —
  carry an *owned* handle through the macro with a `type_map` entry that maps it
  to `long`:

  ```rust
  native_method! {
      java_type = "example.Foo",
      type_map = { unsafe JOwned<Box<String>> => long },
      static fn create(value: JString) -> JOwned<Box<String>>,
  }
  ```

  The `unsafe` mapping asserts the handle is layout-compatible with `jlong`
  (it is — the jnisafe types are `#[repr(transparent)]` over a `jlong`), and the
  macro emits the real `JOwned<…>` type into your impl. See
  [`native_method.rs`](example/rust/src/native_method.rs) and
  [`bind_type.rs`](example/rust/src/bind_type.rs).

  > **Borrowed handles (`JRef` / `JMut`)** carry a `'local` lifetime that can't be
  > named from these macros' `const`-evaluated context, so use `#[jni_mangle]`
  > for borrow parameters; `native_method!` / `bind_java_type!` cover owned
  > handles and plain JNI types.

* **Overloaded natives** are handled too. When a class declares two natives of
  the same name, both the JVM and the checker mangle each to the long
  `Java_pkg_Class_name__<args>` form; the checker derives the Rust-side argument
  descriptor from your signatures so the two overloads pair up cleanly. See
  [`overloaded.rs`](example/rust/src/overloaded.rs).

* **`bind_java_type! { methods { … } fields { … } constructors { … } }`** — the
  *Rust→Java* direction: these clauses generate type-safe wrappers that call
  into Java. The checker verifies each named method/field/constructor exists on
  the bound class with a matching receiver (static/instance) and JVM descriptor —
  a missing or mis-typed member is reported (E040–E044) instead of failing at
  runtime. If the bound class isn't passed to `--java`, the binding can't be
  verified and a warning (W004) is emitted. See
  [`bind_type.rs`](example/rust/src/bind_type.rs).

### 4. Check that the two sides agree

```bash
jnisafe-check \
  --rust-crate path/to/your/crate \
  --java path/to/classes-or.jar
```

`--java` accepts a `.class` file, a directory of classes, or a `.jar`, and is repeatable.
Add `[--format human|json]` and `[--quiet]` to control output.
Exit codes: **0** clean, **1** mismatches found, **3** internal error.

Wire this into CI to catch signature/ownership drift before it becomes a runtime crash.

## Building from source

To build the workspace, run the test suite, or publish releases, see [development.md](development.md).

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/license/mit)

at your option.
