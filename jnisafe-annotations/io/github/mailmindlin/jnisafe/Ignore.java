package io.github.mailmindlin.jnisafe;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * Excludes a {@code native} method from jnisafe-check.
 *
 * <p>Apply it to a {@code native} method that has no corresponding
 * {@code Java_*} Rust export, or whose signature the checker cannot or should
 * not verify (for example a method implemented by a different toolchain). The
 * checker skips annotated methods entirely rather than reporting a missing
 * export.
 *
 * @see Owned
 * @see Ref
 * @see Mut
 */
@Target({ElementType.METHOD})
@Retention(RetentionPolicy.CLASS)
public @interface Ignore {
}
