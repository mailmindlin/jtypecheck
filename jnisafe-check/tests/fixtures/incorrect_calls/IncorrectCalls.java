package example;

/**
 * A plain Java class (no native methods) used as the Rust→Java call target for
 * the deliberately-wrong bindings in
 * {@code jnisafe-check/tests/fixtures/incorrect_calls/wrong.rs}.
 *
 * <p>The members below all exist; the Rust {@code bind_java_type!} references
 * them with the wrong name, receiver, type, or signature to exercise the
 * reverse-direction diagnostics E040–E044 (plus a binding to a class that is
 * never passed to {@code --java}, which yields W004).
 */
public class IncorrectCalls {
    static int realMethod(int x) {
        return x;
    }

    int instanceValue;

    IncorrectCalls(int x) {
        instanceValue = x;
    }
}
