package example;

import io.github.mailmindlin.jnisafe.Owned;

/**
 * The {@link NativeMethod} owned-handle subset ({@code create}/{@code drop})
 * again, implemented in Rust via the {@code bind_java_type! { … }} macro's
 * {@code native_methods { … }} block (Java→Rust). As with {@link NativeMethod},
 * the jnisafe {@code @Owned} handle is carried through the macro by a
 * {@code type_map = { unsafe JOwned<Box<String>> => long }} entry, and the
 * checker validates each native against its Rust impl. (The borrowed
 * {@code tryGet}/{@code get}/{@code set} methods are out of reach here for the
 * same lifetime reason as {@link NativeMethod} — use {@link Mangle} for those.)
 *
 * <p>What's unique to {@code bind_java_type!} is the <em>Rust→Java</em>
 * direction: the plain Java members below ({@code doubled}, {@code counter},
 * the {@code BindType(int)} constructor) are call targets declared by the same
 * macro's {@code methods}/{@code fields}/{@code constructors} clauses, and the
 * checker verifies they exist with the expected signatures. The {@code roundTrip}
 * native drives all three at runtime (see {@code example/rust/src/bind_type.rs}).
 */
public class BindType {
    private static native @Owned("Box<String>") long create(String value);
    private static native void drop(@Owned("Box<String>") long ptr);

    // Calls back into the Java members below through the generated Rust→Java
    // wrappers, returning the resulting counter so main() can observe it.
    private static native int roundTrip(int x);

    // Rust→Java call targets (referenced by the macro's methods/fields/constructors).
    static int counter;

    BindType(int value) {
        counter += value;
    }

    static int doubled(int x) {
        return x * 2;
    }
}
