package example;

import io.github.mailmindlin.jnisafe.Owned;

/**
 * The {@link HandWritten} contract, implemented in Rust with the
 * {@code native_method! { }} macro and registered via {@code RegisterNatives}.
 * The owned jnisafe handle flows through the macro using a
 * {@code type_map = { unsafe JOwned<Box<String>> => long }} entry, so the Rust
 * impl keeps its {@code JOwned} type and the checker still validates the
 * {@code @Owned} annotations here.
 *
 * <p>Only the owned-handle subset ({@code create}/{@code drop}) appears here:
 * the borrowed {@code @Ref}/{@code @Mut} methods ({@code tryGet}/{@code get}/
 * {@code set}) need a {@code 'local} lifetime that the macro's const context
 * can't name — use {@code #[jni_mangle]} for those, as in {@link Mangle}.
 */
public class NativeMethod {
    private static native @Owned("Box<String>") long create(String value);
    private static native void drop(@Owned("Box<String>") long ptr);
}
