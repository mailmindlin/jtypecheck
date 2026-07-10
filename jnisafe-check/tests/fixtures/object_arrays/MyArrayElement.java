package example;

// A plain user class used as the element type of a `Pose[]` native return/param
// in `ObjectArrays`. Paired with tests/fixtures/object_arrays/natives.rs, whose
// Rust side names it through a `bind_java_type!`-bound wrapper `JPose`.
public final class MyArrayElement {}
