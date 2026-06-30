package example;

import io.github.mailmindlin.jnisafe.Owned;

/**
 * W013: the class holds an {@code @Owned} field but no method ever consumes it —
 * there is no disposal path, so the handle it holds will leak. Compare with
 * {@link OwnedFieldDisposed}, which has a {@code close()} that frees it.
 */
public class OwnedFieldLeak {
    @Owned("Box<String>") long handle;

    private static native @Owned("Box<String>") long wrap(String s);

    // No W012: in a constructor the field starts uninitialized (0), so this is
    // not overwriting a live handle.
    OwnedFieldLeak() {
        this.handle = wrap("x");
    }
}
