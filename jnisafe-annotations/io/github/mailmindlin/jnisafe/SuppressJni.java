package io.github.mailmindlin.jnisafe;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * Opts a Java element out of one or more {@code jnisafe-check} handle-flow
 * diagnostics. The {@link #value()} holds the diagnostic <em>categories</em> to
 * silence on the annotated element (and everything lexically inside it).
 *
 * <p>The checker reads this from the compiled {@code .class} bytecode, so unlike
 * {@link java.lang.SuppressWarnings} (which is {@code @Retention(SOURCE)} and
 * never reaches the class file) it must be retained at {@code CLASS} level. The
 * {@code Jni} suffix keeps the simple name distinct from both
 * {@code java.lang.SuppressWarnings} and {@code kotlin.Suppress}.
 *
 * <p>Recognised categories (Rust-flavoured, with descriptive synonyms):
 * <ul>
 *   <li>{@code "forge"} — a handle fabricated from a non-handle value (E060)</li>
 *   <li>{@code "transmute"} (alias {@code "type"}) — a handle used as the wrong
 *       Rust pointee type (E061)</li>
 *   <li>{@code "alias"} — borrow/move violations: ref used mutably (E062),
 *       use-after-move (E063), field not cleared after a take (E064), and an
 *       exclusive handle aliased across one call's arguments (E065)</li>
 *   <li>{@code "forget"} (alias {@code "leak"}) — an owned handle or field that is
 *       never consumed (W011, W012, W013)</li>
 *   <li>{@code "expose"} — a handle on a public/protected surface (W014)</li>
 *   <li>{@code "annotate"} — an unannotated handle local (W010)</li>
 *   <li>{@code "all"} — every category</li>
 * </ul>
 *
 * <p>Class- and method-level suppression is read reliably; a {@code TYPE_USE}
 * placement on a local is best-effort (it rides the same local type-annotation
 * table as a local {@code @Owned}, which not every JVM compiler emits).
 */
@Target({
    ElementType.TYPE,
    ElementType.METHOD,
    ElementType.CONSTRUCTOR,
    ElementType.FIELD,
    ElementType.TYPE_USE,
})
@Retention(RetentionPolicy.CLASS)
public @interface SuppressJni {
    /**
     * The diagnostic categories to silence on the annotated element.
     *
     * @return one or more category keys (see the type-level documentation)
     */
    String[] value();
}
