package io.github.mailmindlin.jnisafe;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * Marks a {@code long} that carries an <em>owned</em> Rust pointer, corresponding
 * to the {@code JOwned<T>} wrapper on the Rust side.
 *
 * <p>Use it where ownership of the pointer crosses the JNI boundary: as the
 * return type of a {@code native} method that hands a freshly allocated object
 * to Java, or on a by-value parameter that consumes the pointer (for example a
 * {@code drop} method that takes ownership back so Rust can free it).
 *
 * <p>Unlike {@link Ref} and {@link Mut}, {@code nullable} is not cross-checked
 * for {@code @Owned}: {@code JOwned} is internally nullable, so it is never
 * wrapped in {@code Option} on the Rust side.
 *
 * @see Ref
 * @see Mut
 */
@Target({ElementType.TYPE_USE})
@Retention(RetentionPolicy.CLASS)
public @interface Owned {
    /**
     * The Rust pointee type the {@code long} stands for, e.g. {@code "Box<String>"}.
     * Whitespace is ignored, so {@code "Box<String>"} and {@code "Box < String >"}
     * compare equal to the Rust side.
     *
     * @return the Rust type, as written in source
     */
    String value();

    /**
     * Whether {@code null} (a {@code 0} handle) is a valid value. Not enforced for
     * {@code @Owned}; present only so the three pointer annotations share a shape.
     *
     * @return {@code true} if null is permitted (the default)
     */
    boolean nullable() default true;
}
