package example;

import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

/**
 * Each native here is paired with a deliberately-wrong macro-based Rust impl in
 * {@code jnisafe-check/tests/fixtures/incorrect_macros/wrong.rs}, so the checker
 * reports exactly one diagnostic per method — demonstrating that macro-declared
 * natives are validated just like hand-written {@code Java_*} exports. The
 * expected code and the macro used are noted in each comment.
 */
public class IncorrectMacros {
    // E023 (#[jni_mangle]): Java @Ref, Rust JOwned.
    private static native void kindMismatch(@Ref(value = "Box<String>", nullable = false) long ptr);

    // W003 (#[jni_mangle]): Rust returns a borrow handle (JRef). nullable=false
    // isolates the borrow-return warning from an E03x finding.
    private static native @Ref(value = "Box<String>", nullable = false) long borrowReturn();

    // E024 (native_method!): Box<String> (Java) vs Box<u32> (Rust).
    private static native void typeMismatch(@Owned("Box<String>") long ptr);

    // E021 (bind_java_type!): int (Java) vs jlong (Rust).
    private static native void primMismatch(int value);
}
