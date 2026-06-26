## Set up repo

Install [pixi](https://pixi.sh), then:

```bash
pixi install          # fetch the pinned Rust + JDK toolchain into ./.pixi
pixi run build        # cargo build --workspace + the annotations JAR
pixi run test         # regenerate Java fixtures, then cargo test --workspace
pixi run demo         # run the checker against example/rust — prints "ok: ..."
```

### Tasks

| Task | Does |
|------|------|
| `pixi run build` | Build the Rust workspace **and** `build/jnisafe-annotations.jar` |
| `pixi run test` | Regenerate `.class` test fixtures, then run all Cargo tests |
| `pixi run demo` | End-to-end: compile the example Java and check it against `example/rust` |
| `pixi run check-jni -- <args>` | Run the checker CLI on your own inputs (see below) |
| `pixi run jar` | Build only the distributable annotations JAR |
| `pixi run fixtures` | Only regenerate `jnisafe-check/tests/fixtures/classes/*.class` |
| `pixi run fmt` / `lint` | `cargo fmt --all` / `cargo clippy --workspace --all-targets` |
| `pixi run clean` | `cargo clean` + remove `build/` |

Tasks declare their inputs/outputs, so pixi skips steps whose inputs are unchanged.

## Repository layout

Two directories hold example/test material, split by intent:

- **[`example/`](example/)** — the correct, user-facing example: a small Java API
  ([`example/java`](example/java/example)) and a complete, compiling Rust
  implementation ([`example/rust`](example/rust)) covering each binding style
  (hand-written `Java_*`, `#[jni_mangle]`, `native_method!`, `bind_java_type!`,
  overloads). This is "here's how you use jnisafe" — keep new positive material
  here.
- **[`jnisafe-check/tests/fixtures/`](jnisafe-check/tests/fixtures)** — the
  negative / exhaustive cases that drive `tests/e2e.rs` and the `demo`. This is
  where the broken-on-purpose code lives.

## Test fixtures

Each fixture is a directory pairing a Java class with the Rust meant to bind to
it, so the checker has something concrete to accept or reject:

| Directory          | Java                   | Rust         | Asserts (`tests/e2e.rs`)                              |
| ------------------ | ---------------------- | ------------ | ----------------------------------------------------- |
| `incorrect/`       | `Incorrect.java`       | `wrong.rs`   | Hand-written `Java_*` exports, one diagnostic per method |
| `incorrect_macros/`| `IncorrectMacros.java` | `wrong.rs`   | Macro-declared natives across the three macro forms   |
| `incorrect_calls/` | `IncorrectCalls.java`  | `wrong.rs`   | Rust→Java call bindings (methods/fields/constructors, E040–E044/W004) |
| `field_handles/`   | `FieldHandles.java`    | `wrong.rs`   | Handle fields: `@Owned`/`@Ref`/`@Mut` annotation vs Rust handle type (E045 mismatch, W005 unannotated) |
| `overloaded/`      | *(uses `Overloaded`)*  | `natives.rs` | **Positive:** overloaded `native_method!` natives resolve cleanly |

The `*.rs` files are **not compiled** — the Rust loader parses them with `syn`
(it only ever reads `*.rs`, so the co-located `.java` is ignored). In the
`wrong.rs` cases each function reproduces its Java counterpart's mangled symbol
but disagrees in exactly one way, so the checker emits one diagnostic per case;
the expected code is documented in the Java file's comments. `overloaded/` is a
*correct* case (hence `natives.rs`, not `wrong.rs`) — a minimal isolated crate so
overload resolution is tested without the other example natives; its Java class,
`Overloaded`, is a correct example under `example/java`.

### Generated `.class` files

`jnisafe-check/tests/fixtures/classes/example/*.class` is compiled from the
fixture `*.java` plus the correct classes in `example/java` (all in the `example`
package). It is **gitignored** — a build artifact, not a source. The
`@Ref`/`@Mut`/`@Owned` annotations are `@Retention(CLASS)` + `@Target(TYPE_USE)`,
so they ride along as `RuntimeInvisibleTypeAnnotations` in the bytecode.

`pixi run test` regenerates these before the suite runs. To produce them without
running the tests (e.g. before a bare `cargo test`), run `pixi run fixtures` —
the tests fail with an actionable hint if they are missing.

## Toolchain

pixi pins everything from conda-forge (see [`pixi.toml`](pixi.toml)):

- **Rust** >= 1.89 (hard floor)
- **JDK** >= 11 (might be compatible with 8)

`pixi.lock` is committed so every machine and CI run resolves the same versions.

## Publishing

### Rust crates → crates.io

`jnisafe` and `jnisafe-check` are independent (neither depends on the other), so each publishes on its own:

```bash
cargo publish -p jnisafe
cargo publish -p jnisafe-check        # users then `cargo install jnisafe-check`
```

Before the first publish, each crate's `Cargo.toml` needs `description`, `license`, and
`repository` fields (crates.io rejects publishes without them). `example/rust` is marked
`publish = false`. CI has a manual **Release crates** workflow
([`.github/workflows/release.yml`](.github/workflows/release.yml)) that runs
`cargo publish` for the crate you pick, using a `CARGO_REGISTRY_TOKEN` secret.

### Java annotations → Maven Central

Maven/Gradle users consume dependencies from a Maven *repository*, not a loose jar — so
the annotations JAR is published to **[Maven Central](https://central.sonatype.com)** via
[`jnisafe-annotations/pom.xml`](jnisafe-annotations/pom.xml). That POM is used **only** for
publishing — day-to-day builds still use plain `javac`/`jar` (`pixi run jar`).

```bash
pixi run package-annotations   # build jar + sources + javadoc (local sanity check, no signing)
pixi run publish-annotations   # GPG-sign + deploy to Central (needs key + credentials)
```

In CI, the manual **Release annotations** workflow
([`.github/workflows/release-annotations.yml`](.github/workflows/release-annotations.yml))
runs the deploy using `MAVEN_CENTRAL_USERNAME` / `MAVEN_CENTRAL_PASSWORD` (a Central Portal
token), `GPG_PRIVATE_KEY`, and `MAVEN_GPG_PASSPHRASE` secrets.

The publish **groupId** is `io.github.mailmindlin`.