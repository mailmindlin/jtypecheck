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