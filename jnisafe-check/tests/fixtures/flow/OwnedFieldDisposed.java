package example;

import io.github.mailmindlin.jnisafe.Owned;

/**
 * Control for W013: an {@code @Owned} field that <em>is</em> consumed by a
 * disposal method (and the field cleared afterwards), so the class has a valid
 * disposal path and the flow pass reports nothing.
 */
public class OwnedFieldDisposed {
    @Owned("Box<String>") long handle;

    private static native @Owned("Box<String>") long wrap(String s);

    private static native void drop(@Owned("Box<String>") long s);

    // No W012: in a constructor the field starts uninitialized (0).
    OwnedFieldDisposed() {
        this.handle = wrap("x");
    }

    void close() {
        // Ok: swap before consume
        @Owned("Box<String>") long handle = this.handle;
        this.handle = 0;
        drop(handle);
    }

    void badClose() {
        // Error: consumed before cleared
        drop(this.handle);
        this.handle = 0;
    }
}
