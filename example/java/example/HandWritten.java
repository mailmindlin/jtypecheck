package example;

import io.github.mailmindlin.jnisafe.Mut;
import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

/**
 * The baseline create/tryGet/get/set/drop contract over an {@code @Owned}
 * {@code Box<String>} handle. Implemented in Rust as hand-written {@code Java_*}
 * exports (see {@code example/rust/src/hand_written.rs}). {@link Mangle}
 * implements the identical contract with {@code #[jni_mangle]} — diff the two
 * Rust files to see what the attribute changes (only the export, not the types
 * or bodies).
 */
public class HandWritten {
    private static native @Owned("Box<String>") long create(String value);
    private static native String tryGet(@Ref("Box<String>") long ptr);
    private static native String get(@Ref(value = "Box<String>", nullable = false) long ptr);
    private static native void set(@Mut(value = "Box<String>", nullable = false) long ptr, String value);
    private static native void drop(@Owned("Box<String>") long ptr);
}
