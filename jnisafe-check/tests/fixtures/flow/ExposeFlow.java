package example;

import io.github.mailmindlin.jnisafe.Mut;
import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

/**
 * Encapsulation: a raw handle must not escape on a public/protected surface.
 * Kept declaration-only (no field-mutating bodies) so every finding here is a
 * W014 — the owned-field move/leak rules are exercised by {@link FieldTake},
 * {@link OwnedFieldLeak}, and {@link OwnedFieldDisposed}. Fields are {@code @Ref}
 * (a borrow carries no disposal obligation, so no W013).
 */
public class ExposeFlow {
    // W014: a handle on a public field.
    public @Ref("Box<String>") long exposed;

    // No W014: a private field is fine.
    private @Ref("Box<String>") long hidden;

    private static native @Owned("Box<String>") long wrap(String s);

    // W014 (return) + W014 (parameter): a handle escapes on both surfaces.
    public static native @Owned("Box<String>") long clone(@Ref("Box<String>") long src);

    // W014: a handle on a public parameter.
    public static native void makeUppercase(@Mut("Box<String>") long ptr);

    // W014: a handle on a public parameter.
    public static native void drop(@Owned("Box<String>") long ptr);

    // W014: a public method returns a handle (a fresh one — nothing escapes a field).
    public @Owned("Box<String>") long take() {
        return wrap("x");
    }
}
