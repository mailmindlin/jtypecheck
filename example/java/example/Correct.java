package example;

import io.github.mailmindlin.jnisafe.Mut;
import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

public class Correct {
    private static native @Owned("Box<String>") long create(String value);
    private static native String tryGet(@Ref("Box<String>") long ptr);
    private static native String get(@Ref(value = "Box<String>", nullable = false) long ptr);
    private static native void set(@Mut(value = "Box<String>", nullable = false) long ptr, String value);
    private static native void drop(@Owned("Box<String>") long ptr);
}
