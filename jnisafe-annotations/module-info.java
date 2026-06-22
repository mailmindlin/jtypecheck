/**
 * Annotations marking JNI pointer ownership ({@code @Owned}, {@code @Ref},
 * {@code @Mut}) and check exclusion ({@code @Ignore}), consumed by jnisafe-check
 * to verify a Rust/Java JNI layer agrees on names, signatures, and pointer types.
 */
module io.github.mailmindlin.jnisafe {
    exports io.github.mailmindlin.jnisafe;
}
