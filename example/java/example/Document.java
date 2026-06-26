package example;

import io.github.mailmindlin.jnisafe.Mut;
import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

/**
 * A stateful object whose Rust {@code Box<String>} lives in a Java field across
 * calls — the field-handle counterpart to the parameter-only {@link HandWritten}
 * contract. By extending {@link NativeObject}, the handle is created once
 * ({@code @Owned}), borrowed immutably under the read lock ({@code @Ref}) and
 * mutably under the write lock ({@code @Mut}), and freed once on {@link #close()}
 * ({@code @Owned}). The native methods themselves are the ordinary jnisafe
 * parameter path (see {@code example/rust/src/document.rs}); soundness across
 * threads comes from routing every call through the inherited lock.
 */
public final class Document extends NativeObject {
    public Document(String initial) {
        super(create(initial));
    }

    /** Read the current text (shared borrow). */
    public String text() {
        return nativeRead(Document::get);
    }

    /** Append to the text (exclusive borrow). */
    public void appendText(String suffix) {
        nativeWrite(ptr -> Document.append(ptr, suffix));
    }

    @Override
    protected void destroy(long ptr) {
        Document.drop(ptr);
    }

    private static native @Owned("Box<String>") long create(String value);

    private static native String get(@Ref(value = "Box<String>", nullable = false) long ptr);

    private static native void append(@Mut(value = "Box<String>", nullable = false) long ptr, String suffix);

    private static native void drop(@Owned("Box<String>") long ptr);
}
