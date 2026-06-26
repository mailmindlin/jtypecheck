package example;

import io.github.mailmindlin.jnisafe.Mut;
import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

/**
 * The {@link HandWritten} contract again, but implemented in Rust with the jni
 * {@code #[jni_mangle("example.Mangle")]} attribute instead of a hand-written
 * {@code Java_*} export. The Java side is unchanged: jnisafe still sees
 * {@code @Owned}/{@code @Ref}/{@code @Mut} pointer handles, because
 * {@code #[jni_mangle]} only renames the export — the Rust signature (and its
 * {@code JOwned}/{@code JRef}/{@code JMut} types) is untouched.
 */
public class Mangle {
    private static native @Owned("Box<String>") long create(String value);
    private static native String tryGet(@Ref("Box<String>") long ptr);
    private static native String get(@Ref(value = "Box<String>", nullable = false) long ptr);
    // setValue exercises the snake_case (set_value) -> lowerCamelCase mangling.
    private static native void setValue(@Mut(value = "Box<String>", nullable = false) long ptr, String value);
    private static native void drop(@Owned("Box<String>") long ptr);
}
