package example;

import io.github.mailmindlin.jnisafe.Mut;
import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

/** Exclusive-borrow aliasing within a single call. */
public class AliasFlow {
    private static native @Owned("Box<u32>") long make();

    private static native void assign(@Mut("Box<u32>") long dst, @Ref("Box<u32>") long src);

    private static native void dropU32(@Owned("Box<u32>") long s);

    // E065: the same handle is passed as the @Mut (exclusive) argument and the
    // @Ref argument of one call — a mutable borrow may not be aliased.
    void aliasMutRef() {
        @Owned("Box<u32>") long p = make();
        assign(p, p);
        dropU32(p);
    }
}
