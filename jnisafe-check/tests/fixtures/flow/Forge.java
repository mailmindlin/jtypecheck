package example;

import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

/** Pointer forging: a handle slot fed a value that has no handle provenance. */
public class Forge {
    private static native void dropString(@Owned("Box<String>") long s);

    private static native void print(@Ref(value = "Box<String>", nullable = false) long s);

    // E060: a fabricated constant flows into a handle slot.
    void fabricate() {
        long p = 12345L;
        dropString(p);
    }

    // E060: the literal 0 (null) into a non-nullable @Ref slot.
    void nullIntoNonNullable() {
        print(0L);
    }

    // No E060: the literal 0 is a valid null pointer
    void nullIntoNullable() {
        dropString(0L);
    }

    // E060: the parameter is not a valid handle
    void externalFabricate(long param) {
        dropString(param);
    }
}
