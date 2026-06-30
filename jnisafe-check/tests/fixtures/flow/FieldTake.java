package example;

import io.github.mailmindlin.jnisafe.Owned;

/** Owned-field move discipline: taking from a field must clear it; overwriting
 * a live owned field without disposing the old one leaks it. */
public class FieldTake {
    @Owned("Box<String>") long handle;

    private static native @Owned("Box<String>") long wrap(String s);

    private static native void drop(@Owned("Box<String>") long s);

    FieldTake(boolean initialize) {
        if (initialize) {
            // No W012: starts at 0 in constructor
            this.handle = wrap("ctor");
        }
    }

    // E064: the owned handle is taken out of the field but the field is not
    // cleared (left dangling, aliasing a freed pointer).
    void takeWithoutClear() {
        drop(this.handle);
    }

    // W012: the field is overwritten while it still could hold a live owned handle
    void overwriteOnce() {
        this.handle = wrap("y");
    }

    // No W012: checked by an if statement
    void overwriteChecked() {
        if (this.handle == 0) {
            // handle is null, no overwrite
            this.handle = wrap("y");
        }
    }

    // No W012: assert means it's safe
    void overwriteAssert() {
        assert this.handle == 0;
        // handle is null, no overwrite
        this.handle = wrap("y");
    }

    // W012: the field is overwritten while it still holds a live owned handle,
    // without the old handle being disposed first.
    void overwriteLive() {
        this.handle = wrap("y");
        this.handle = wrap("z");
    }

    // E060: arithmetic forges the handle — `this.handle++` reads the pointer,
    // adds 1, and stores the non-handle result back into the handle field.
    void mutate() {
        this.handle++;
    }

    // E064: the owned handle escapes via the return without the field being
    // cleared first, so the field is left aliasing a handle the caller now owns.
    @Owned("Box<String>") long takeViaReturn() {
        return this.handle;
    }
}
