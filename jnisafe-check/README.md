# jnisafe-check

A static checker that verifies a Rust/Java JNI layer agrees — on symbol names,
FFI signatures (arity and parameter types), Java object types, and the
ownership of `long`-as-pointer handles — before the two sides drift apart and
crash at runtime.

You annotate the Java side with `@Owned` / `@Ref` / `@Mut` (from
[`jnisafe-annotations`]), implement the natives in Rust with the [`jnisafe`]
wrapper types, and run this checker in CI to catch any mismatch.

## Install

```bash
cargo install jnisafe-check
```

## Use

```bash
jnisafe-check \
  --rust-crate path/to/your/crate \
  --java path/to/classes-or.jar
```

`--java` accepts a `.class` file, a directory of classes, or a `.jar`, and is
repeatable. Add `--format human|json` and `--quiet` to control output.

Exit codes: **0** clean, **1** mismatches found, **3** internal error.

See the [project README] for the end-to-end workflow and a worked example.

[`jnisafe-annotations`]: https://github.com/mailmindlin/jtypecheck/tree/main/jnisafe-annotations
[`jnisafe`]: https://crates.io/crates/jnisafe
[project README]: https://github.com/mailmindlin/jtypecheck

## License

Licensed under either of Apache-2.0 or MIT at your option.
