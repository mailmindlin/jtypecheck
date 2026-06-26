package example;

import io.github.mailmindlin.jnisafe.Owned;

/** Owned-field move discipline: taking from a field must clear it; overwriting
 * a live owned field without disposing the old one leaks it. */
public class FieldTake {
    @Owned("Box<String>") long handle;

    private static native @Owned("Box<String>") long wrap(String s);

    private static native void drop(@Owned("Box<String>") long s);

    // E064: the owned handle is taken out of the field but the field is not
    // cleared (left dangling, aliasing a freed pointer).
    void takeWithoutClear() {
        drop(this.handle);
    }

    // W012: the field is overwritten while it still holds a live owned handle,
    // without the old handle being disposed first.
    void overwriteLive() {
        this.handle = wrap("y");
        this.handle = wrap("z");
    }
}
