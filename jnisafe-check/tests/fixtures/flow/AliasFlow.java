package example;

import io.github.mailmindlin.jnisafe.Mut;
import io.github.mailmindlin.jnisafe.Owned;
import io.github.mailmindlin.jnisafe.Ref;

/** Exclusive-borrow aliasing within a single call. */
public class AliasFlow {
    private static native @Owned("Box<u32>") long make();

    private static native void assign(@Mut("Box<u32>") long dst, @Ref("Box<u32>") long src);

    private static native void dropU32(@Owned("Box<u32>") long s);

    void aliasMutRef() {
        @Owned("Box<u32>") long p = make();
        // E065: the same handle is passed as the @Mut (exclusive) argument and the
        // @Ref argument of one call — a mutable borrow may not be aliased.
        assign(p, p);
        dropU32(p);
    }

    void aliasMutRefCFG(boolean unknown) {
        @Owned("Box<u32>") long p = make();
        @Owned("Box<u32>") long q = make();
        // E065: the same handle is passed as the @Mut (exclusive) argument and the
        // @Ref argument of one call — a mutable borrow may not be aliased.
        assign(p, unknown ? p : q);
        dropU32(p);
        dropU32(q);
    }
}
