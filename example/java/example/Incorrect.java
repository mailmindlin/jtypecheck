package example;

import io.github.mailmindlin.jnisafe.Mut;
import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

/**
 * Each native method here is paired with a deliberately-wrong Rust export in
 * {@code jnisafe-check/tests/fixtures/incorrect/wrong.rs} (except the ones that
 * are intentionally missing), so the checker reports exactly one diagnostic
 * per method. The expected code for each case is noted in its comment below.
 */
public class Incorrect {
    // E001: no Rust export at all (this instance method is unmangled-matched
    // against a symbol that wrong.rs does not define).
    private native @Ref("Box<String>") long createWrongType();

    // E023: Java says @Ref, Rust returns JOwned.
    private static native void kindMismatch(@Ref("Box<String>") long ptr);

    // E024: Box<String> (Java) vs Box<u32> (Rust). nullable=false isolates the
    // type mismatch (both sides are @Ref/JRef, so a default-nullable annotation
    // would also trip the E025 nullability check).
    private static native void typeMismatch(@Ref(value = "Box<String>", nullable = false) long ptr);

    // E025: nullable=false (Java) vs Option<JRef<..>> (Rust).
    private static native void nullMismatch(@Ref(value = "Box<String>", nullable = false) long ptr);

    // E020: String (Java object) vs jint (Rust primitive) — category mismatch.
    private static native void objMismatch(String value);

    // E021: int (Java) vs jlong (Rust) — primitive kind mismatch.
    private static native void primMismatch(int value);

    // E010: two Java params vs one Rust param (after env+class).
    private static native void arityMismatch(@Ref("Box<String>") long ptr, String value);

    // E002: instance (non-static) method, but Rust takes JClass.
    private native void recvMismatch();
}
