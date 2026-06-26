package example;

import java.util.concurrent.locks.ReentrantReadWriteLock;
import java.util.function.LongConsumer;
import java.util.function.LongFunction;

/**
 * Base class for a Java object that owns a Rust value across calls, holding it
 * as an opaque {@code long} handle (a jnisafe {@code JOwned} pointer) in a
 * field. This is the <em>sound</em> way to make jnisafe's smart-pointer wrappers
 * work for fields: the handle lives on the Java side, and every native access
 * goes through this class's read/write lock, so a validated pointer only ever
 * reaches Rust <em>as a parameter, under a held lock</em> — exactly the contract
 * the parameter path already relies on.
 *
 * <p>The lock maps Rust's borrow rules onto the runtime:
 * <ul>
 *   <li>{@link #nativeRead} takes the <b>read</b> lock (shared) — pass the
 *       handle to a native taking {@code @Ref} ({@code JRef}, a shared
 *       {@code &T}).</li>
 *   <li>{@link #nativeWrite} takes the <b>write</b> lock (exclusive) — pass the
 *       handle to a native taking {@code @Mut} ({@code JMut}, a {@code &mut T}),
 *       or {@code @Owned} when consuming it.</li>
 *   <li>{@link #close} takes the write lock, nulls the handle, and frees it
 *       exactly once; it is idempotent and any later access throws
 *       {@link NativeObjectReleasedException}.</li>
 * </ul>
 *
 * <p><b>Concurrency caveat.</b> The read lock permits <em>concurrent</em>
 * readers, so a handle borrowed via {@link #nativeRead} can yield {@code &T} on
 * several threads at once. That is sound only if the Rust pointee is {@code Sync}
 * — which an {@code Arc<T: Send + Sync>} guarantees but a {@code Box<T>} does
 * not. Use {@code Arc}-backed handles for shared ({@code @Ref}) access and
 * reserve {@code Box}-backed handles ({@code @Mut}) for {@link #nativeWrite}.
 *
 * <p>This is a prototype helper living in the example package; it is intended to
 * be promoted to a published {@code io.github.mailmindlin.jnisafe} runtime
 * artifact once the API settles.
 */
public abstract class NativeObject implements AutoCloseable {
    private final ReentrantReadWriteLock lock = new ReentrantReadWriteLock();

    /** The native handle; {@code 0} once {@link #close} has run. */
    private long ptr;

    /**
     * @param ptr a non-zero handle, typically the {@code @Owned} {@code long}
     *            returned by a {@code create} native
     * @throws NullPointerException if {@code ptr} is {@code 0}
     */
    protected NativeObject(long ptr) {
        if (ptr == 0) {
            throw new NullPointerException("native handle must be non-zero");
        }
        this.ptr = ptr;
    }

    /**
     * Run {@code fn} with the handle under the shared read lock. Use for natives
     * that borrow the pointee immutably ({@code @Ref}).
     *
     * @throws NativeObjectReleasedException if this object has been closed
     */
    protected final <R> R nativeRead(LongFunction<R> fn) {
        var read = lock.readLock();
        read.lock();
        try {
            long p = this.ptr;
            if (p == 0) {
                throw new NativeObjectReleasedException();
            }
            return fn.apply(p);
        } finally {
            read.unlock();
        }
    }

    /**
     * Run {@code fn} with the handle under the exclusive write lock. Use for
     * natives that mutate the pointee ({@code @Mut}).
     *
     * @throws NativeObjectReleasedException if this object has been closed
     */
    protected final void nativeWrite(LongConsumer fn) {
        var write = lock.writeLock();
        write.lock();
        try {
            long p = this.ptr;
            if (p == 0) {
                throw new NativeObjectReleasedException();
            }
            fn.accept(p);
        } finally {
            write.unlock();
        }
    }

    /**
     * Free the native handle, exactly once. Takes the write lock so no read or
     * write is in flight, nulls the handle so later access throws, then hands
     * the still-valid pointer to {@link #destroy}. Idempotent.
     */
    @Override
    public final void close() {
        var write = lock.writeLock();
        write.lock();
        try {
            long p = this.ptr;
            if (p == 0) {
                return; // already closed
            }
            this.ptr = 0;
            destroy(p);
        } finally {
            write.unlock();
        }
    }

    /**
     * Release the native allocation behind {@code ptr}. Called by {@link #close}
     * under the write lock with a guaranteed-valid handle; implement it with an
     * {@code @Owned} native (e.g. {@code drop}) so Rust reclaims and frees it.
     */
    protected abstract void destroy(long ptr);

    /** Thrown when a {@link NativeObject} is used after {@link #close}. */
    public static final class NativeObjectReleasedException extends IllegalStateException {
        NativeObjectReleasedException() {
            super("use of a closed NativeObject");
        }
    }
}
