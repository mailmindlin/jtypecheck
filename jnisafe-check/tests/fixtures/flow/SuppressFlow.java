package example;

import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.SuppressJni;

/**
 * {@code @SuppressJni} opt-out: {@code suppressed()} and {@code active()} contain
 * the identical wrong-type violation (E061), but only {@code active()} reports it
 * — the {@code "transmute"} category is silenced on {@code suppressed()}.
 */
public class SuppressFlow {
    private static native @Owned("Box<String>") long wrap(String s);

    private static native void dropI32(@Owned("Box<i32>") long s);

    @SuppressJni("transmute")
    void suppressed() {
        @Owned("Box<String>") long ptr = wrap("x");
        dropI32(ptr);
    }

    void active() {
        @Owned("Box<String>") long ptr = wrap("y");
        dropI32(ptr);
    }
}
