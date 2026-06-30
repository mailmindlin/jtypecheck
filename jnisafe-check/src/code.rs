//! Owned, `cafebabe`-free bytecode model for the handle-flow analysis.
//!
//! `java_loader` lowers each method's decoded `Code` attribute into a
//! [`MethodCode`] — an offset-ordered list of [`Insn`]s plus the line / local
//! tables — so [`cfg`](crate::cfg) and [`flow`](crate::flow) can work on owned
//! data without holding the borrowed parsed class.
//!
//! The operand stack is modelled one entry *per value* (a `long`/`double` is a
//! single entry with `wide = true`), so each [`Op`] records its value-level
//! stack effect. Opcodes irrelevant to handles collapse into [`Op::Other`] with
//! just their pop/push counts, keeping the abstract stack balanced.

use crate::ir::Pointer;

/// A resolved member reference — the target of an `invoke*` or field op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRef {
    /// Internal binary class name, e.g. `example/Flow` (or `[I` for an array).
    pub class: String,
    pub name: String,
    pub descriptor: String,
}

/// One instruction, classified for handle-flow.
#[derive(Debug, Clone)]
pub struct Insn {
    /// Byte offset of this opcode within the `Code` (matches branch targets).
    pub offset: u32,
    /// Source line from the `LineNumberTable`, if present.
    pub line: Option<u32>,
    pub op: Op,
}

/// An instruction's effect. Stack effects are in *values* (a wide value is one).
#[derive(Debug, Clone)]
pub enum Op {
    /// `lconst_0/1` or `ldc2_w` of a `long` — a long constant; `zero` flags `0`.
    LongConst {
        zero: bool,
    },
    /// A `long` produced by arithmetic/conversion (`ladd`, `lneg`, `i2l`, …):
    /// has no handle provenance. Pops `pops` values, pushes one wide long.
    LongCompute {
        pops: u8,
    },
    /// `lload n` — push the long in local `n`.
    LoadLong(u16),
    /// `lstore n` — pop a long into local `n`.
    StoreLong(u16),
    /// `aload n` — push the object ref in local `n` (slot 0 is `this`).
    LoadRef(u16),
    /// `astore n` — pop an object ref into local `n`.
    StoreRef(u16),
    /// `getfield` — pop the receiver, push the field value (`wide` per its type).
    GetField {
        field: MemberRef,
        wide: bool,
    },
    /// `putfield` — pop the value then the receiver.
    PutField {
        field: MemberRef,
        wide: bool,
    },
    /// `getstatic` — push the field value.
    GetStatic {
        field: MemberRef,
        wide: bool,
    },
    /// `putstatic` — pop the value.
    PutStatic {
        field: MemberRef,
        wide: bool,
    },
    /// Any `invoke*`. Pops the args (and the receiver unless `is_static`), then
    /// pushes the return value if non-void.
    Invoke {
        target: MemberRef,
        is_static: bool,
        /// Per-argument width, in declaration order.
        arg_widths: Vec<bool>,
        /// Return width: `None` for `void`, else `Some(wide)`.
        ret: Option<bool>,
    },
    /// `lcmp` — pop two longs, push their `-1`/`0`/`1` comparison (an int). Kept
    /// distinct from [`Op::Other`] so the flow analysis can recognise the
    /// `field == 0` idiom (`getfield; lconst_0; lcmp; ifeq/ifne`).
    LongCmp,
    /// Unconditional jump to an absolute offset.
    Goto(u32),
    /// Conditional branch: pops `pops` value(s); falls through or jumps to `target`.
    Branch {
        target: u32,
        pops: u8,
        kind: BranchKind,
    },
    /// `tableswitch`/`lookupswitch`: pops the key, jumps to one of `targets`.
    Switch {
        targets: Vec<u32>,
    },
    /// `*return` (`pops == 1`) or `return` (`pops == 0`) — exits the method.
    Return {
        pops: u8,
    },
    /// `athrow` — pops the exception, transfers to a handler or exits.
    Athrow,
    Dup,
    Dup2,
    DupX1,
    DupX2,
    Dup2X1,
    Dup2X2,
    Pop,
    Pop2,
    Swap,
    /// Everything else: pop `pops` values, push one entry per `pushes` width.
    Other {
        pops: u8,
        pushes: Vec<bool>,
    },
}

/// Which zero-comparison a conditional [`Op::Branch`] performs, for the
/// path-sensitive field-nullness refinement. `IfEq`/`IfNe` are the `== 0` / `!= 0`
/// branches (the only ones we refine on); everything else is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    /// `ifeq` — branch taken when the popped int is `0`.
    IfEq,
    /// `ifne` — branch taken when the popped int is non-`0`.
    IfNe,
    /// Any other conditional branch (`iflt`, `if_icmp*`, `ifnull`, …).
    Other,
}

/// A try/catch range (absolute offsets); `handler` is the catch entry.
#[derive(Debug, Clone)]
pub struct ExceptionRange {
    pub start: u32,
    pub end: u32,
    pub handler: u32,
}

/// A local variable's name over a live range (from `LocalVariableTable`).
#[derive(Debug, Clone)]
pub struct LocalName {
    pub slot: u16,
    pub start: u32,
    pub end: u32,
    pub name: String,
}

/// A `@Ref`/`@Mut`/`@Owned` annotation on a *local* over a live range
/// (recovered from the `Code` attribute's `localvar_target` type annotations).
#[derive(Debug, Clone)]
pub struct LocalHandleAnn {
    pub slot: u16,
    pub start: u32,
    pub end: u32,
    pub ptr: Pointer,
}

/// A method parameter's slot + handle contract, for seeding the entry frame.
#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub slot: u16,
    pub wide: bool,
    /// The `@Ref`/`@Mut`/`@Owned` contract, if this parameter is a handle.
    pub handle: Option<Pointer>,
}

/// The analyzable body of one method.
#[derive(Debug, Clone)]
pub struct MethodCode {
    pub max_locals: u16,
    /// Offset-ordered instructions.
    pub insns: Vec<Insn>,
    pub exceptions: Vec<ExceptionRange>,
    pub local_names: Vec<LocalName>,
    pub local_handles: Vec<LocalHandleAnn>,
    /// Parameters in declaration order, with their local slots.
    pub params: Vec<ParamInfo>,
}

impl MethodCode {
    /// The index into [`insns`](Self::insns) for a given byte offset, if any.
    pub fn index_of(&self, offset: u32) -> Option<usize> {
        self.insns.binary_search_by_key(&offset, |i| i.offset).ok()
    }

    /// The declared handle annotation covering `slot` at instruction `offset`.
    pub fn local_handle_at(&self, slot: u16, offset: u32) -> Option<&Pointer> {
        self.local_handles
            .iter()
            .find(|a| a.slot == slot && offset >= a.start && offset < a.end)
            .map(|a| &a.ptr)
    }

    /// `true` if any `@Ref`/`@Mut`/`@Owned` local annotation covers `slot` at
    /// `offset` (used by the unannotated-local lint, which only needs presence).
    pub fn local_annotated_at(&self, slot: u16, offset: u32) -> bool {
        self.local_handle_at(slot, offset).is_some()
    }

    /// The local variable name for `slot` live at `offset`, if `-g` recorded it.
    pub fn local_name_at(&self, slot: u16, offset: u32) -> Option<&str> {
        self.local_names
            .iter()
            .find(|n| n.slot == slot && offset >= n.start && offset < n.end)
            .map(|n| n.name.as_str())
    }
}

/// Parse a JVM method descriptor into `(arg widths, return width)`. A `long` or
/// `double` argument/return is `wide`; `void` returns `None`. Used to model the
/// stack effect of `invoke*` against a callee's descriptor string.
pub fn parse_method_descriptor(desc: &str) -> (Vec<bool>, Option<bool>) {
    let bytes = desc.as_bytes();
    let mut i = 0;
    let mut args = Vec::new();
    // Skip the leading '('.
    if bytes.first() == Some(&b'(') {
        i = 1;
    }
    while i < bytes.len() && bytes[i] != b')' {
        let (wide, next) = scan_field_type(bytes, i);
        args.push(wide);
        i = next;
    }
    // Skip ')'.
    if i < bytes.len() && bytes[i] == b')' {
        i += 1;
    }
    let ret = if i < bytes.len() && bytes[i] == b'V' {
        None
    } else if i < bytes.len() {
        let (wide, _) = scan_field_type(bytes, i);
        Some(wide)
    } else {
        None
    };
    (args, ret)
}

/// Scan one field type starting at `i`, returning `(is_wide, next_index)`.
fn scan_field_type(bytes: &[u8], i: usize) -> (bool, usize) {
    match bytes.get(i) {
        Some(b'[') => {
            // An array is a (narrow) reference regardless of element type.
            let (_, next) = scan_field_type(bytes, i + 1);
            (false, next)
        }
        Some(b'L') => {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b';' {
                j += 1;
            }
            (false, j + 1) // past the ';'
        }
        Some(b'J' | b'D') => (true, i + 1), // long / double are wide
        Some(_) => (false, i + 1),          // Z B C S I F
        None => (false, i),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_arg_and_return_widths() {
        // (long, String) -> void
        assert_eq!(
            parse_method_descriptor("(JLjava/lang/String;)V"),
            (vec![true, false], None)
        );
        // (int) -> long
        assert_eq!(parse_method_descriptor("(I)J"), (vec![false], Some(true)));
        // () -> long
        assert_eq!(parse_method_descriptor("()J"), (vec![], Some(true)));
        // (double, int[]) -> Object
        assert_eq!(
            parse_method_descriptor("(D[I)Ljava/lang/Object;"),
            (vec![true, false], Some(false))
        );
        // () -> void
        assert_eq!(parse_method_descriptor("()V"), (vec![], None));
    }
}
