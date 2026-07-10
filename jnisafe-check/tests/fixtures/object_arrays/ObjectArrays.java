package example;

// Positive fixture: native methods whose parameter/return arrays have a *user
// class* element type (`Pose[]`). The Rust side (tests/fixtures/object_arrays/
// natives.rs) names the element through a `bind_java_type!`-bound wrapper
// `JPose` → `example.Pose`, so `JObjectArray<'local, JPose<'local>>` must lower
// to `[Lexample/Pose;` and match cleanly. See the e2e test
// `object_array_of_bound_type_matches`.
public final class ObjectArrays {
    // Pose[]  <-> JObjectArray<'local, JPose<'local>>   (as a return type)
    public static native MyArrayElement[] makePoses(int n);

    // Pose[]  <-> JObjectArray<'local, JPose<'local>>   (as a parameter)
    public static native int countPoses(MyArrayElement[] poses);
}
