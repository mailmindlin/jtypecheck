package example;

import io.github.mailmindlin.jnisafe.Owned;

/** Encapsulation: a raw handle must not escape on a public/protected surface. */
public class ExposeFlow {
    // W014: a handle on a public field.
    public @Owned("Box<String>") long exposed;

    // No W014: a private field is fine.
    private @Owned("Box<String>") long hidden;

    private static native @Owned("Box<String>") long wrap(String s);

    // W014: a public method returns a handle.
    public @Owned("Box<String>") long take() {
        return wrap("x");
    }
}
