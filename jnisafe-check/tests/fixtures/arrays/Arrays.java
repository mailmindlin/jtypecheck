package example;

// Positive fixture: native methods whose array parameters and returns exercise
// the generic jni array wrappers on the Rust side — `JPrimitiveArray<T>` and
// `JObjectArray<E>` (including the default `Object` element and a nested array).
// Paired with tests/fixtures/arrays/natives.rs; the checker must find every
// method matched cleanly, with no diagnostics. See the e2e test
// `array_wrappers_match_their_java_declarations`.
public final class Arrays {
    // int[]      <-> JPrimitiveArray<'local, jint>
    public static native int sumInts(int[] values);

    // byte[]     <-> JPrimitiveArray<'local, jbyte>   (as a return type)
    public static native byte[] makeBytes(int len);

    // String[]   <-> JObjectArray<'local, JString<'local>>
    public static native int countStrings(String[] names);

    // Object[]   <-> JObjectArray<'local>             (default JObject element)
    public static native void inspect(Object[] items);

    // byte[][]   <-> JObjectArray<'local, JByteArray<'local>>  (nested array)
    public static native int totalLen(byte[][] chunks);
}
