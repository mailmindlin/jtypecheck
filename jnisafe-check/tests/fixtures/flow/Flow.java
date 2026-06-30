package example;

import java.util.Objects;

import io.github.mailmindlin.jnisafe.Mut;
import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

/**
 * The motivating examples for the Java-side handle-flow analysis. Each method
 * isolates exactly one diagnostic; the expected code is noted in its comment.
 * These are self-contained — every {@code native} they call is declared here —
 * so the flow pass resolves contracts without cross-class loading.
 */
public class Flow {
    private static native @Owned("Box<String>") long wrapString(String s);

    private static native void set(@Mut("Box<String>") long s, String value);

    private static native void print(@Ref("Box<String>") long s);

    private static native void dropString(@Owned("Box<String>") long s);

    private static native void dropI32(@Owned("Box<i32>") long s);

    private static native String get(@Ref("Box<String>") long s);

    // W010: the local holds a handle but is not annotated. Consumed so the only
    // finding is the missing annotation, not a leak.
    void test1() {
        long ptr = wrapString("foo");
        dropString(ptr);
    }

    // W011: an owned handle that is never consumed before the method returns.
    void test2() {
        @Owned("Box<String>") long ptr = wrapString("bar");
    }

    // E061: the handle is Box<String>, but dropI32 consumes a Box<i32>.
    void test3() {
        @Owned("Box<String>") long ptr = wrapString("bar");
        dropI32(ptr);
    }

    // E062: a borrowed (@Ref) handle passed where a mutable borrow (@Mut) is required.
    void test4(@Ref("Box<String>") long param) {
        set(param, "baz");
    }

    // E063: the handle is used after it was consumed by dropString.
    void test5() {
        @Owned("Box<String>") long ptr = wrapString("qux");
        dropString(ptr);
        String value = get(ptr);
    }

    // W011: an owned handle that is never consumed before the method returns.
    void test6(String param) {
        @Owned("Box<String>") long ptr = wrapString("bar");
        try {
            Objects.requireNonNull(param);
            dropString(ptr);
        } catch (Exception e) {
            // ptr not consumed
        }
    }
}
