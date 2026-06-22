# Test fixtures

## `classes/` — compiled Java native-method declarations

Prebuilt `.class` files for `example.Correct` and `example.Incorrect`, committed
so the test suite is hermetic (no `javac` at test time). The `@Ref`/`@Mut`/
`@Owned` annotations are `@Retention(CLASS)` + `@Target(TYPE_USE)`, so they are
embedded as `RuntimeInvisibleTypeAnnotations` in the bytecode.

Regenerate after changing the example Java or the annotations:

```sh
# from the jnisafe-check/ crate root:
javac -d /tmp/ann ../jnisafe-annotations/io/github/mailmindlin/jnisafe/*.java
javac -cp /tmp/ann -d tests/fixtures/classes ../example/java/example/*.java
```

## `incorrect/wrong.rs` — deliberately-wrong Rust exports

NOT compiled — parsed by the `syn` Rust loader as a fixture. Each function
reproduces an `Incorrect.java` native method's mangled symbol but disagrees in
exactly one way, so the checker emits one diagnostic per case. Each case's expected diagnostic code is documented in the `Incorrect.java` comments.
