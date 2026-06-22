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
pub extern "system" fn Java_example_Correct_create(/* … */) -> JOwned<Box<String>> { /* … */ }
```

Can you get around it? Sure. But it's better than nothing.

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
See [`example/rust`](example/rust/src/lib.rs) for a complete, compiling implementation.

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
