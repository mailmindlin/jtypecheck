package example;

/**
 * Two overloaded native methods. Because they share a name, the JNI spec (and
 * {@code java_loader}) mangles each to the long {@code ..._combine__<args>} form.
 * The Rust loader's {@code resolve_overloads} pass re-mangles the matching
 * macro-declared natives to the same long form, so both sides pair up cleanly —
 * implemented in {@code example/rust/src/overloaded.rs} and exercised by
 * the {@code overloaded_macro_methods_match_when_supported} e2e test.
 */
public class Overloaded {
    private static native int combine(int a, int b);
    private static native int combine(String a, String b);
}
