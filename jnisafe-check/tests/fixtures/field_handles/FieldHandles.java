package example;

import io.github.mailmindlin.jnisafe.Owned;

/**
 * Field-handle cross-check fixture, paired with
 * {@code jnisafe-check/tests/fixtures/field_handles/wrong.rs}. A {@code long}
 * field that the Rust side declares as a handle (via {@code bind_java_type!}'s
 * {@code fields { … }}) must carry a matching {@code @Owned}/{@code @Ref}/{@code @Mut}
 * annotation, since a bare {@code long} is indistinguishable from any other
 * handle on the wire:
 *
 * <ul>
 *   <li>{@code cached} — annotated correctly → clean</li>
 *   <li>{@code bare} — stores a handle but is unannotated → W005</li>
 *   <li>{@code wrong} — annotated with the wrong pointee type → E045</li>
 * </ul>
 */
public class FieldHandles {
    @Owned("Box<String>") long cached;
    long bare;
    @Owned("Box<u64>") long wrong;
}
