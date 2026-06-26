# Test fixtures

The negative / exhaustive cases that drive `tests/e2e.rs` and the `demo`. Each
subdirectory pairs a deliberately-broken (or, for `overloaded/`, a deliberately-
correct) Java class with the Rust meant to bind to it.

For the correct, user-facing example see the top-level
[`example/`](../../../example/) directory instead.

The fixture conventions, the parse-only `wrong.rs` files, and how the gitignored
`classes/*.class` are generated are documented in
[development.md](../../../development.md#test-fixtures).
