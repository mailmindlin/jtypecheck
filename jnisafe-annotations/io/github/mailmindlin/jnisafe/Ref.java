package io.github.mailmindlin.jnisafe;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * Marks a {@code long} that carries an <em>immutably borrowed</em> Rust pointer,
 * corresponding to the {@code JRef<T>} wrapper on the Rust side.
 *
 * <p>Use it on a {@code native} method parameter when Java lends the pointer to
 * Rust for the duration of the call without transferring ownership and without
 * permitting mutation. Pair it with {@link Mut} when the call needs to mutate
 * the pointee, or {@link Owned} when ownership crosses the boundary.
 *
 * @see Mut
 * @see Owned
 */
@Target({ElementType.TYPE_USE})
@Retention(RetentionPolicy.CLASS)
public @interface Ref {
    /**
     * The Rust pointee type the {@code long} stands for, e.g. {@code "Box<String>"}.
     * Whitespace is ignored, so {@code "Box<String>"} and {@code "Box < String >"}
     * compare equal to the Rust side.
     *
     * @return the Rust type, as written in source
     */
    String value();

    /**
     * Whether {@code null} (a {@code 0} handle) is a valid value.
     *
     * <p>A nullable {@code @Ref} maps to Rust {@code Option<JRef<..>>} (where
     * {@code 0} decodes to {@code None}); {@code nullable = false} maps to a bare
     * {@code JRef<..>}. The checker reports a mismatch if the two sides disagree,
     * because a {@code 0} handle reaching a bare {@code JRef} is immediate
     * undefined behaviour.
     *
     * @return {@code true} if null is permitted (the default)
     */
    boolean nullable() default true;
}
