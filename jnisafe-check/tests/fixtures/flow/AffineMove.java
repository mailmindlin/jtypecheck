package example;

import io.github.mailmindlin.jnisafe.Owned;

/** Affine ("non-copy") ownership: assigning an owned handle moves it. */
public class AffineMove {
    private static native @Owned("Box<String>") long wrap(String s);

    private static native void drop(@Owned("Box<String>") long s);

    // E063: `b = a` moves the owned handle out of `a`; using `a` afterwards is a
    // use-after-move.
    void doubleUse() {
        @Owned("Box<String>") long a = wrap("a");
        @Owned("Box<String>") long b = a;
        drop(a);
        drop(b);
    }
}
