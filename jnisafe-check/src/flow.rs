//! Java-side intraprocedural handle-flow analysis.
//!
//! Where [`check`](crate::check) validates the JNI *boundary* (that each Java
//! `native` method agrees with its Rust export), this module looks *inside* Java
//! types and method bodies and tracks the `long` handle values as they flow —
//! catching forging, wrong-type and exclusive-borrow violations, use-after-move,
//! leaks, and handle exposure.
//!
//! It works on compiled bytecode (via `cafebabe`), so it is language-agnostic
//! across the JVM (Java, Kotlin, Scala, …) and needs no `javac`. The analysis is
//! a forward abstract interpreter over the per-instruction CFG ([`cfg`]); see the
//! design doc for the lattice, transfer functions, and documented soundness gaps
//! (intraprocedural; no heap alias analysis; single-threaded).

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;

use crate::cfg::Cfg;
use crate::code::{Insn, MethodCode, Op};
use crate::diagnostics::{Diagnostic, Report};
use crate::ir::{JavaLoc, Pointer, PointerKind};
use crate::java_loader::{self, FlowClass, FlowField, FlowMethod, JavaLoadError};

/// Analyze the given Java inputs (`.class` files, directories, or `.jar`s) for
/// handle-flow violations, pushing diagnostics into `report`.
pub fn analyze(java: &[PathBuf], report: &mut Report) -> Result<(), JavaLoadError> {
    let classes = java_loader::load_flow(java)?;
    let contracts = Contracts::build(&classes);
    for class in &classes {
        check_exposed_handles(class, report);

        // The owned handle fields of this class, by name → their pointer type.
        let owned_fields: HashMap<String, Pointer> = class
            .fields
            .iter()
            .filter_map(|f| f.handle.clone().map(|p| (f.name.clone(), p)))
            .collect();

        // Fields consumed (moved to a disposing sink) by *any* method — the input
        // to the whole-class W013 disposal-path check.
        let mut consumed: BTreeSet<String> = BTreeSet::new();
        for method in &class.methods {
            if let Some(code) = &method.code {
                analyze_method(
                    class,
                    method,
                    code,
                    &contracts,
                    &owned_fields,
                    &mut consumed,
                    report,
                );
            }
        }

        // W013 — an `@Owned` field that no method ever consumes has no disposal
        // path, so whatever handle it holds will leak.
        for f in &class.fields {
            let owned = f
                .handle
                .as_ref()
                .is_some_and(|p| p.kind == PointerKind::Owned);
            if owned && !consumed.contains(&f.name) {
                gated(
                    report,
                    &[&f.suppressed[..], &class.suppressed[..]],
                    Diagnostic::warning("W013", "owned handle field is never disposed")
                        .with_java(Some(field_loc(class, f)))
                        .note(
                            "no method in this class consumes this `@Owned` field (e.g. a \
                             `close()`/`dispose()` that passes it to an owning native), so the \
                             handle it holds leaks",
                        ),
                );
            }
        }
    }
    Ok(())
}

// ===========================================================================
// Call/return contracts
// ===========================================================================

/// The handle contract of a method: which parameters / return are handles. Used
/// to type-check call arguments and to seed a fresh handle from a call's result.
struct Contract {
    params: Vec<Option<Pointer>>,
    ret: Option<Pointer>,
}

/// All loaded methods' contracts, keyed by `(class, name, descriptor)` — the
/// exact identity an `invoke*` carries in bytecode, so no overload guessing.
struct Contracts {
    by_key: HashMap<(String, String, String), Contract>,
}

impl Contracts {
    fn build(classes: &[FlowClass]) -> Self {
        let mut by_key = HashMap::new();
        for class in classes {
            for m in &class.methods {
                by_key.insert(
                    (
                        class.internal_name.clone(),
                        m.name.clone(),
                        m.descriptor.clone(),
                    ),
                    Contract {
                        params: m.params.clone(),
                        ret: m.ret.clone(),
                    },
                );
            }
        }
        Contracts { by_key }
    }

    fn get(&self, class: &str, name: &str, descriptor: &str) -> Option<&Contract> {
        // A two-step borrow-free lookup without allocating the tuple key.
        self.by_key
            .iter()
            .find(|((c, n, d), _)| c == class && n == name && d == descriptor)
            .map(|(_, v)| v)
    }
}

// ===========================================================================
// Abstract domain
// ===========================================================================

/// The abstract value of a slot/local. A `long` that carries a handle is
/// [`Handle`](AbsVal::Handle); a `long` without handle provenance is
/// [`Forged`](AbsVal::Forged); `0` is [`Zero`](AbsVal::Zero) (a candidate null).
#[derive(Debug, Clone, PartialEq)]
enum AbsVal {
    /// A non-`long` value, or a value we don't track.
    NotHandle,
    /// The current method's receiver (`this`) — tracked so `this.<field>` field
    /// accesses can be told apart from accesses on some other object.
    This,
    /// The `<...>.$assertionsDisabled` flag. Modelled as known-*false* (i.e.
    /// assertions enabled), so the `ifne` that skips an `assert` is never taken
    /// and the asserted invariant holds on the live path.
    AssertFlag,
    /// The `int` result of comparing field `f` to `0` (`getfield f; lconst_0;
    /// lcmp`), so a following `ifeq`/`ifne` can refine `f`'s null-state per edge.
    CmpFieldZero(String),
    /// The literal `0` long — a valid null only where nullability allows.
    Zero,
    /// A `long` with no handle provenance (a constant, arithmetic result, or an
    /// unannotated incoming `long`). Forged if it reaches a handle sink.
    Forged,
    /// A tracked handle.
    Handle(Handle),
    /// Join of conflicting values — suppresses further per-value errors.
    Top,
}

#[derive(Debug, Clone, PartialEq)]
struct Handle {
    ptr: Pointer,
    state: HandleState,
    /// Must-alias identity: the storage the handle was last loaded from, or the
    /// call site that produced it. `None` once a control-flow merge joins two
    /// differently-sourced handles — we keep the type/kind/state but lose the
    /// single identity, so the exclusive-aliasing check can't claim an alias on a
    /// merely-possible (one-path) overlap.
    id: Option<HandleId>,
    /// Sticky provenance: the owned field this handle was originally read out of
    /// (survives copies into locals). Drives the must-clear check (E064) and the
    /// whole-class disposal-path check (W013).
    from_field: Option<String>,
}

/// The null-state of a tracked `this.<field>` handle field within a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldState {
    /// Definitely `0` (a constructor's fresh field, after a clear, or under a
    /// proven `field == 0` guard) — overwriting it is safe.
    Null,
    /// Definitely holds a handle — overwriting it without disposal leaks it.
    Live,
    /// Could be either (a non-constructor's field at entry, or a merge of the
    /// two) — conservatively treated as possibly-live for the overwrite check.
    Unknown,
}

fn join_field(a: FieldState, b: FieldState) -> FieldState {
    if a == b { a } else { FieldState::Unknown }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum HandleState {
    Live,
    /// Consumed / moved out — using it again is a use-after-move (E063).
    Moved,
}

/// A handle's must-alias identity. Two operands carrying the *same* `HandleId` at
/// one call site are provably the same handle — enough for the single-call-site
/// exclusive-borrow check (E065) without any heap alias analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HandleId {
    /// Loaded from local slot `n`.
    Local(u16),
    /// Read from `this.<field>` (used once field tracking lands).
    #[allow(dead_code)]
    Field(String),
    /// A fresh result produced by the `invoke` at this byte offset.
    Fresh(u32),
}

/// One operand-stack entry. `wide` marks a `long`/`double` (two JVM slots) so the
/// width-sensitive shuffles (`dup2`/`pop2`/…) stay balanced.
#[derive(Debug, Clone, PartialEq)]
struct Slot {
    val: AbsVal,
    wide: bool,
}

/// The abstract machine frame: an operand stack, the locals array, and the
/// null-state of each tracked `this.<field>` handle field.
#[derive(Debug, Clone, PartialEq)]
struct Frame {
    stack: Vec<Slot>,
    locals: Vec<AbsVal>,
    fields: HashMap<String, FieldState>,
}

impl Frame {
    fn push(&mut self, val: AbsVal, wide: bool) {
        self.stack.push(Slot { val, wide });
    }

    fn pop(&mut self) -> Slot {
        self.stack.pop().unwrap_or(Slot {
            val: AbsVal::NotHandle,
            wide: false,
        })
    }

    fn local(&self, slot: u16) -> AbsVal {
        self.locals
            .get(slot as usize)
            .cloned()
            .unwrap_or(AbsVal::NotHandle)
    }

    fn set_local(&mut self, slot: u16, val: AbsVal) {
        let i = slot as usize;
        if i >= self.locals.len() {
            self.locals.resize(i + 1, AbsVal::NotHandle);
        }
        self.locals[i] = val;
    }

    /// The null-state of field `name` (an untracked field reads as `Unknown`).
    fn field(&self, name: &str) -> FieldState {
        self.fields
            .get(name)
            .copied()
            .unwrap_or(FieldState::Unknown)
    }

    fn set_field(&mut self, name: &str, st: FieldState) {
        self.fields.insert(name.to_owned(), st);
    }
}

/// Lattice join of two abstract values. Equal values join to themselves; two
/// handles of the *same* pointer type and state but different origins keep the
/// type/state and drop to an unknown identity (`id = None`); anything else
/// (differing type, `Live ⊔ Moved`, handle ⊔ non-handle) joins to `Top`, which
/// never itself errors — avoiding false positives at merges.
fn join_val(a: &AbsVal, b: &AbsVal) -> AbsVal {
    if a == b {
        return a.clone();
    }
    if let (AbsVal::Handle(ha), AbsVal::Handle(hb)) = (a, b)
        && ha.ptr == hb.ptr
        && ha.state == hb.state
    {
        return AbsVal::Handle(Handle {
            ptr: ha.ptr.clone(),
            state: ha.state,
            id: if ha.id == hb.id { ha.id.clone() } else { None },
            from_field: if ha.from_field == hb.from_field {
                ha.from_field.clone()
            } else {
                None
            },
        });
    }
    AbsVal::Top
}

/// Join `new` into `dst`; returns `true` if `dst` changed. Frames at a merge have
/// equal stack height (the verifier guarantees it); if they ever differ we keep
/// the existing frame rather than risk an unbalanced join.
fn merge(dst: &mut Option<Frame>, new: &Frame) -> bool {
    match dst {
        None => {
            *dst = Some(new.clone());
            true
        }
        Some(cur) => {
            if cur.stack.len() != new.stack.len() {
                return false;
            }
            let mut changed = false;
            for (c, n) in cur.stack.iter_mut().zip(&new.stack) {
                let j = join_val(&c.val, &n.val);
                if j != c.val {
                    c.val = j;
                    changed = true;
                }
            }
            if cur.locals.len() < new.locals.len() {
                cur.locals.resize(new.locals.len(), AbsVal::NotHandle);
            }
            for (i, n) in new.locals.iter().enumerate() {
                let j = join_val(&cur.locals[i], n);
                if j != cur.locals[i] {
                    cur.locals[i] = j;
                    changed = true;
                }
            }
            // Both frames descend from the same (fully-seeded) entry, so their
            // field keys match; a missing key still reads as `Unknown`.
            for (name, &st) in &new.fields {
                let slot = cur
                    .fields
                    .entry(name.clone())
                    .or_insert(FieldState::Unknown);
                let j = join_field(*slot, st);
                if j != *slot {
                    *slot = j;
                    changed = true;
                }
            }
            changed
        }
    }
}

// ===========================================================================
// Per-method analysis
// ===========================================================================

#[allow(clippy::too_many_arguments)]
fn analyze_method(
    class: &FlowClass,
    method: &FlowMethod,
    code: &MethodCode,
    contracts: &Contracts,
    owned_fields: &HashMap<String, Pointer>,
    consumed: &mut BTreeSet<String>,
    report: &mut Report,
) {
    let cfg = Cfg::new(code);
    let n = code.insns.len();
    if n == 0 {
        return;
    }

    // A constructor's `@Owned` fields start uninitialised (`0`); any other method
    // inherits whatever a prior call left, so they start `Unknown`.
    let is_ctor = method.name == "<init>";
    let initial_field = if is_ctor {
        FieldState::Null
    } else {
        FieldState::Unknown
    };
    let fields: HashMap<String, FieldState> = owned_fields
        .keys()
        .map(|name| (name.clone(), initial_field))
        .collect();

    // Seed the entry frame: locals from the parameters (a handle parameter starts
    // Live; an unannotated `long` parameter has no provenance → Forged), and slot 0
    // is the receiver `this` for an instance method.
    let mut entry = Frame {
        stack: Vec::new(),
        locals: vec![AbsVal::NotHandle; code.max_locals as usize],
        fields,
    };
    if !method.is_static {
        entry.set_local(0, AbsVal::This);
    }
    for p in &code.params {
        let v = match &p.handle {
            Some(ptr) => AbsVal::Handle(Handle {
                ptr: ptr.clone(),
                state: HandleState::Live,
                id: Some(HandleId::Local(p.slot)),
                from_field: None,
            }),
            None if p.wide => AbsVal::Forged,
            None => AbsVal::NotHandle,
        };
        entry.set_local(p.slot, v);
    }

    // Forward worklist fixpoint over the per-instruction CFG.
    let mut state: Vec<Option<Frame>> = vec![None; n];
    state[0] = Some(entry);
    let mut work: VecDeque<usize> = VecDeque::from([0]);
    while let Some(idx) = work.pop_front() {
        let frame = match &state[idx] {
            Some(f) => f.clone(),
            None => continue,
        };
        let out = transfer(&frame, &code.insns[idx], contracts, owned_fields);

        if let Op::Branch { target, kind, .. } = &code.insns[idx].op {
            // A conditional branch may refine a field's null-state differently on
            // its two edges (the `field == 0` idiom), and a known-false condition
            // (`$assertionsDisabled`) prunes one edge entirely.
            let cond = frame.stack.last().map(|s| &s.val);
            let (fallthrough, taken) = branch_edges(cond, *kind);
            let next = idx + 1;
            if next < n {
                propagate(&mut state, &mut work, next, &out, fallthrough.as_ref());
            }
            if let Some(t) = code.index_of(*target) {
                propagate(&mut state, &mut work, t, &out, taken.as_ref());
            }
        } else {
            for s in cfg.normal_succ(idx) {
                if merge(&mut state[s], &out) {
                    work.push_back(s);
                }
            }
        }

        let handlers = cfg.exc_succ(idx);
        if !handlers.is_empty() {
            // On the exception edge the JVM clears the operand stack and pushes
            // the exception; locals/fields are as they were when the throw occurred.
            let handler_frame = Frame {
                stack: vec![Slot {
                    val: AbsVal::NotHandle,
                    wide: false,
                }],
                locals: frame.locals.clone(),
                fields: frame.fields.clone(),
            };
            for s in handlers {
                if merge(&mut state[s], &handler_frame) {
                    work.push_back(s);
                }
            }
        }
    }

    // Emission pass: re-examine each reachable sink against its converged entry
    // state (so each diagnostic is emitted exactly once). Findings go to a local
    // report first, then through `@SuppressJni` gating into the real one.
    let mut local = Report::default();
    for (idx, slot) in state.iter().enumerate() {
        let Some(frame) = slot else { continue };
        match &code.insns[idx].op {
            Op::Invoke {
                target,
                is_static,
                arg_widths,
                ..
            } => check_call(
                class, method, code, idx, frame, target, *is_static, arg_widths, contracts,
                consumed, &mut local,
            ),
            Op::StoreLong(n) => {
                check_unannotated_local(class, method, code, idx, frame, *n, &mut local)
            }
            Op::PutField { field, .. } => check_field_store(
                class,
                method,
                code,
                idx,
                frame,
                field,
                owned_fields,
                &mut local,
            ),
            Op::Return { pops: 1 } => {
                check_owned_return(class, method, code, idx, frame, consumed, &mut local)
            }
            _ => {}
        }
    }
    check_leaks(class, method, code, &state, &mut local);

    let supp = [&method.suppressed[..], &class.suppressed[..]];
    for d in local.diagnostics {
        gated(report, &supp, d);
    }
}

/// A per-edge refinement: which field to pin to which null-state along one
/// branch edge. `None` means no refinement.
type Refine = Option<(String, FieldState)>;

/// Merge `out` (with an optional field refinement applied) into successor `s`'s
/// state, scheduling it for re-processing if its state grew. `edge = None` means
/// the edge is dead (a known-false condition) and is skipped.
fn propagate(
    state: &mut [Option<Frame>],
    work: &mut VecDeque<usize>,
    s: usize,
    out: &Frame,
    edge: Option<&Refine>,
) {
    let Some(refine) = edge else { return };
    let mut frame = out.clone();
    if let Some((name, st)) = refine {
        frame.set_field(name, *st);
    }
    if merge(&mut state[s], &frame) {
        work.push_back(s);
    }
}

/// The two outgoing edges of a conditional branch, given the condition value on
/// top of the (pre-transfer) stack: `(fallthrough, taken)`. `None` (the outer
/// `Option`) marks a *dead* edge — pruned because the condition is known.
fn branch_edges(
    cond: Option<&AbsVal>,
    kind: crate::code::BranchKind,
) -> (Option<Refine>, Option<Refine>) {
    use crate::code::BranchKind;
    match (cond, kind) {
        // `field == 0`: `ifne` skips the body, so the taken edge has `field != 0`
        // (Live) and the fall-through has `field == 0` (Null).
        (Some(AbsVal::CmpFieldZero(f)), BranchKind::IfNe) => (
            Some(Some((f.clone(), FieldState::Null))),
            Some(Some((f.clone(), FieldState::Live))),
        ),
        // `ifeq` takes the branch when equal, so the taken edge has `field == 0`.
        (Some(AbsVal::CmpFieldZero(f)), BranchKind::IfEq) => (
            Some(Some((f.clone(), FieldState::Live))),
            Some(Some((f.clone(), FieldState::Null))),
        ),
        // `$assertionsDisabled` is modelled as `false`: `ifne` is never taken
        // (assertions enabled), so the skip-the-assert edge is dead.
        (Some(AbsVal::AssertFlag), BranchKind::IfNe) => (Some(None), None),
        (Some(AbsVal::AssertFlag), BranchKind::IfEq) => (None, Some(None)),
        _ => (Some(None), Some(None)),
    }
}

/// W010 — a `long` local is assigned a tracked handle but carries no
/// `@Owned`/`@Ref`/`@Mut` annotation, so its ownership/borrow intent is undocumented
/// (and escapes the boundary check). Fires at the `lstore` that stores a handle
/// into a slot with no local handle-annotation covering the variable's scope.
/// Best-effort: it relies on `javac` emitting local type annotations (`-g`).
fn check_unannotated_local(
    class: &FlowClass,
    method: &FlowMethod,
    code: &MethodCode,
    idx: usize,
    frame: &Frame,
    slot: u16,
    report: &mut Report,
) {
    // Only a *tracked* handle (from a native, a param, or a copy) is a concern; a
    // forged/zero/ordinary long stored here is caught elsewhere or is irrelevant.
    if !matches!(frame.stack.last().map(|s| &s.val), Some(AbsVal::Handle(_))) {
        return;
    }
    // The local's scope (and any annotation on it) begins *after* the defining
    // store, so probe the next instruction's offset.
    let scope_off = code
        .insns
        .get(idx + 1)
        .map_or(code.insns[idx].offset, |i| i.offset);
    if code.local_annotated_at(slot, scope_off) {
        return;
    }
    let loc = flow_loc(class, method, code, idx);
    let mut d = Diagnostic::warning("W010", "a handle is stored in an unannotated `long` local")
        .with_java(Some(loc))
        .note(
            "annotate the local with `@Owned`/`@Ref`/`@Mut` so its ownership is \
             documented and checked",
        );
    if let Some(name) = code.local_name_at(slot, scope_off) {
        d = d.note(format!("local `{name}`"));
    }
    report.push(d);
}

/// W011 — an `@Owned` handle still `Live` in a local at a (normal) method exit was
/// never consumed, returned, or stored, so it leaks. Only a handle that is
/// *definitely* live at the exit is reported: a handle that is live on one path
/// but moved on another joins to `Top` and is left alone (no false positive).
/// Reported once per leaked local, pinned to the method.
fn check_leaks(
    class: &FlowClass,
    method: &FlowMethod,
    code: &MethodCode,
    state: &[Option<Frame>],
    report: &mut Report,
) {
    let mut leaked: BTreeSet<u16> = BTreeSet::new();
    let mut at: Option<usize> = None;
    for (idx, slot) in state.iter().enumerate() {
        let Some(frame) = slot else { continue };
        if !matches!(&code.insns[idx].op, Op::Return { .. }) {
            continue;
        }
        for (s, val) in frame.locals.iter().enumerate() {
            if let AbsVal::Handle(h) = val
                && h.ptr.kind == PointerKind::Owned
                && h.state == HandleState::Live
            {
                leaked.insert(s as u16);
                at.get_or_insert(idx);
            }
        }
    }
    let Some(idx) = at else { return };
    for s in leaked {
        let loc = flow_loc(class, method, code, idx);
        let mut d = Diagnostic::warning("W011", "an owned handle is leaked")
            .with_java(Some(loc))
            .note(
                "this `@Owned` handle is never consumed (passed to an owning native), \
                 returned, or stored before the method returns",
            );
        if let Some(name) = code.local_name_at(s, code.insns[idx].offset) {
            d = d.note(format!("leaked handle: local `{name}`"));
        }
        report.push(d);
    }
}

/// The pure transfer function used by the fixpoint: apply the instruction's effect
/// to a copy of `frame`. Diagnostics are *not* emitted here (that is the separate
/// emission pass) so re-visiting an instruction never double-reports. The state
/// updates here — affine moves and handle identity — *are* what the emission pass
/// later reads, so they must run inside the fixpoint.
fn transfer(
    frame: &Frame,
    insn: &Insn,
    contracts: &Contracts,
    owned_fields: &HashMap<String, Pointer>,
) -> Frame {
    let mut f = frame.clone();
    match &insn.op {
        Op::LongConst { zero } => f.push(if *zero { AbsVal::Zero } else { AbsVal::Forged }, true),
        Op::LongCompute { pops } => {
            for _ in 0..*pops {
                f.pop();
            }
            f.push(AbsVal::Forged, true);
        }
        Op::LoadLong(n) => {
            // A value loaded from local `n` is, for aliasing, "the handle in `n`".
            let v = reid(f.local(*n), HandleId::Local(*n));
            f.push(v, true);
        }
        Op::StoreLong(n) => {
            let s = f.pop();
            // Affine move: storing a *live owned* handle that came from another
            // local moves it out of that source (so a later use is a use-after-move).
            if let AbsVal::Handle(h) = &s.val
                && h.ptr.kind == PointerKind::Owned
                && h.state == HandleState::Live
                && let Some(HandleId::Local(src)) = h.id
                && src != *n
            {
                mark_local_moved(&mut f, src);
            }
            f.set_local(*n, reid(s.val, HandleId::Local(*n)));
        }
        Op::LoadRef(n) => {
            let v = f.local(*n);
            f.push(v, false);
        }
        Op::StoreRef(n) => {
            let s = f.pop();
            f.set_local(*n, s.val);
        }
        // A read of `this.<owned field>` produces a handle carrying that field's
        // provenance (so a later consume/return can be tied back to the field).
        Op::GetField { field, wide } => {
            let recv = f.pop();
            let v = match owned_fields.get(&field.name) {
                Some(ptr) if recv.val == AbsVal::This => AbsVal::Handle(Handle {
                    ptr: ptr.clone(),
                    state: HandleState::Live,
                    id: Some(HandleId::Field(field.name.clone())),
                    from_field: Some(field.name.clone()),
                }),
                _ => AbsVal::NotHandle,
            };
            f.push(v, *wide);
        }
        // A store to `this.<owned field>` updates the field's tracked null-state
        // (the diagnostics — W012/E060 — fire in the emission pass).
        Op::PutField { field, .. } => {
            let value = f.pop();
            let recv = f.pop();
            if owned_fields.contains_key(&field.name) && recv.val == AbsVal::This {
                let st = match value.val {
                    AbsVal::Zero => FieldState::Null,
                    AbsVal::Handle(_) => FieldState::Live,
                    _ => FieldState::Unknown,
                };
                f.set_field(&field.name, st);
            }
        }
        // `<...>.$assertionsDisabled` is modelled as a known-false flag.
        Op::GetStatic { field, wide } => {
            let v = if field.name == "$assertionsDisabled" {
                AbsVal::AssertFlag
            } else {
                AbsVal::NotHandle
            };
            f.push(v, *wide);
        }
        Op::PutStatic { .. } => {
            f.pop();
        }
        // `lcmp` of `this.<field>` against `0` yields a token a following
        // `ifeq`/`ifne` uses to refine the field's null-state per edge.
        Op::LongCmp => {
            let b = f.pop();
            let a = f.pop();
            let field_zero = |x: &AbsVal, y: &AbsVal| match (x, y) {
                (
                    AbsVal::Handle(Handle {
                        from_field: Some(name),
                        ..
                    }),
                    AbsVal::Zero,
                ) => Some(name.clone()),
                _ => None,
            };
            let cmp = field_zero(&a.val, &b.val).or_else(|| field_zero(&b.val, &a.val));
            f.push(cmp.map_or(AbsVal::NotHandle, AbsVal::CmpFieldZero), false);
        }
        Op::Invoke {
            target,
            is_static,
            arg_widths,
            ret,
        } => {
            let contract = contracts.get(&target.class, &target.name, &target.descriptor);
            // Before popping, note which local-sourced handles this call *consumes*
            // (an `@Owned`-by-value parameter takes ownership) so we can move them out.
            let n = arg_widths.len();
            let base = f.stack.len().saturating_sub(n);
            let mut moves: Vec<u16> = Vec::new();
            if let Some(c) = contract {
                for (i, param) in c.params.iter().enumerate() {
                    if param.as_ref().map(|p| p.kind) != Some(PointerKind::Owned) {
                        continue;
                    }
                    if let Some(Slot {
                        val: AbsVal::Handle(h),
                        ..
                    }) = f.stack.get(base + i)
                        && let Some(HandleId::Local(src)) = h.id
                    {
                        moves.push(src);
                    }
                }
            }
            for _ in 0..n {
                f.pop();
            }
            if !is_static {
                f.pop(); // receiver
            }
            for src in moves {
                mark_local_moved(&mut f, src);
            }
            if let Some(wide) = ret {
                let v = match contract.and_then(|c| c.ret.clone()) {
                    Some(ptr) => AbsVal::Handle(Handle {
                        ptr,
                        state: HandleState::Live,
                        id: Some(HandleId::Fresh(insn.offset)),
                        from_field: None,
                    }),
                    None if *wide => AbsVal::Forged,
                    None => AbsVal::NotHandle,
                };
                f.push(v, *wide);
            }
        }
        Op::Goto(_) => {}
        Op::Branch { pops, .. } => {
            for _ in 0..*pops {
                f.pop();
            }
        }
        Op::Switch { .. } => {
            f.pop(); // the key
        }
        Op::Return { pops } => {
            for _ in 0..*pops {
                f.pop();
            }
        }
        Op::Athrow => {
            f.pop();
        }
        Op::Dup => {
            if let Some(t) = f.stack.last().cloned() {
                f.stack.push(t);
            }
        }
        Op::Dup2 => {
            let top_wide = f.stack.last().map(|s| s.wide).unwrap_or(false);
            if top_wide {
                if let Some(t) = f.stack.last().cloned() {
                    f.stack.push(t);
                }
            } else {
                let len = f.stack.len();
                if len >= 2 {
                    let a = f.stack[len - 2].clone();
                    let b = f.stack[len - 1].clone();
                    f.stack.push(a);
                    f.stack.push(b);
                }
            }
        }
        Op::Pop => {
            f.pop();
        }
        Op::Pop2 => {
            let top_wide = f.stack.last().map(|s| s.wide).unwrap_or(false);
            f.pop();
            if !top_wide {
                f.pop();
            }
        }
        Op::Swap => {
            let len = f.stack.len();
            if len >= 2 {
                f.stack.swap(len - 1, len - 2);
            }
        }
        // The `*_x*` shuffles never appear in our fixtures; approximate them by
        // re-inserting a copy of the top value to keep the stack non-empty. (A
        // documented gap — see the design doc.)
        Op::DupX1 | Op::DupX2 | Op::Dup2X1 | Op::Dup2X2 => {
            if let Some(t) = f.stack.last().cloned() {
                f.stack.push(t);
            }
        }
        Op::Other { pops, pushes } => {
            for _ in 0..*pops {
                f.pop();
            }
            for &wide in pushes {
                f.push(AbsVal::NotHandle, wide);
            }
        }
    }
    f
}

/// Stamp a handle value with a new must-alias identity (non-handles pass through).
fn reid(val: AbsVal, id: HandleId) -> AbsVal {
    match val {
        AbsVal::Handle(mut h) => {
            h.id = Some(id);
            AbsVal::Handle(h)
        }
        other => other,
    }
}

/// Mark the handle in local `slot` as moved-out (affine consume), if it holds one.
fn mark_local_moved(f: &mut Frame, slot: u16) {
    if let Some(AbsVal::Handle(h)) = f.locals.get_mut(slot as usize) {
        h.state = HandleState::Moved;
    }
}

/// The `@SuppressJni` category a diagnostic code belongs to.
fn category(code: &str) -> &'static str {
    match code {
        "E060" => "forge",
        "E061" => "transmute",
        "E062" | "E063" | "E064" | "E065" => "alias",
        "W010" => "annotate",
        "W011" | "W012" | "W013" => "forget",
        "W014" => "expose",
        _ => "",
    }
}

/// Push `d` into `report` unless its category is silenced by any `@SuppressJni`
/// scope — a list of category lists, innermost first (e.g. method-level then
/// class-level). The `"all"` category silences every diagnostic.
fn gated(report: &mut Report, scopes: &[&[String]], d: Diagnostic) {
    let cat = category(&d.code);
    let silenced = scopes
        .iter()
        .flat_map(|s| s.iter())
        .any(|c| c == "all" || c == cat);
    if !silenced {
        report.push(d);
    }
}

/// Check an `invoke` site's arguments against the callee's handle contract,
/// emitting E060 (forging), E061 (wrong type), E062 (ref used mutably),
/// E063 (use-after-move), and E065 (exclusive borrow aliased within one call).
#[allow(clippy::too_many_arguments)]
fn check_call(
    class: &FlowClass,
    method: &FlowMethod,
    code: &MethodCode,
    idx: usize,
    frame: &Frame,
    target: &crate::code::MemberRef,
    _is_static: bool,
    arg_widths: &[bool],
    contracts: &Contracts,
    consumed: &mut BTreeSet<String>,
    report: &mut Report,
) {
    let Some(contract) = contracts.get(&target.class, &target.name, &target.descriptor) else {
        return;
    };
    let argn = arg_widths.len();
    let base = frame.stack.len().saturating_sub(argn);
    for (i, param) in contract.params.iter().enumerate() {
        let Some(expected) = param else { continue };
        let Some(slot) = frame.stack.get(base + i) else {
            continue;
        };
        // Passing an owned field's handle to an owning native disposes it — record
        // that the field has a disposal path (regardless of whether it is the safe
        // shape; an unsafe disposal is still a disposal, caught by E064 below).
        if expected.kind == PointerKind::Owned
            && let AbsVal::Handle(Handle {
                from_field: Some(f),
                ..
            }) = &slot.val
        {
            consumed.insert(f.clone());
        }
        let loc = flow_loc(class, method, code, idx);
        match &slot.val {
            AbsVal::Forged => report.push(
                Diagnostic::error("E060", "a non-handle value is used as a Rust pointer")
                    .with_java(Some(loc))
                    .note(format!(
                        "`{}` expects a {} handle here; only a value from a handle-returning \
                         native, a copy of a handle, or literal `0` (for a nullable slot) is valid",
                        target.name,
                        expected.kind.annotation()
                    )),
            ),
            AbsVal::Zero if !expected.nullable => report.push(
                Diagnostic::error("E060", "literal `0` (null) passed to a non-nullable handle")
                    .with_java(Some(loc))
                    .note(format!(
                        "`{}`'s {} parameter is non-nullable",
                        target.name,
                        expected.kind.annotation()
                    )),
            ),
            AbsVal::Handle(h) if h.state == HandleState::Moved => report.push(
                Diagnostic::error("E063", "handle used after it was consumed").with_java(Some(loc)),
            ),
            AbsVal::Handle(h) if h.ptr.rust_type != expected.rust_type => report.push(
                Diagnostic::error("E061", "handle has the wrong Rust pointee type")
                    .with_java(Some(loc))
                    .expected_found(expected.rust_type.clone(), h.ptr.rust_type.clone()),
            ),
            AbsVal::Handle(h)
                if expected.kind == PointerKind::Mut && h.ptr.kind == PointerKind::Ref =>
            {
                report.push(
                    Diagnostic::error("E062", "a shared (`@Ref`) handle is used mutably")
                        .with_java(Some(loc))
                        .note(format!(
                            "`{}` borrows it as `@Mut`, but it is only an `@Ref`",
                            target.name
                        )),
                )
            }
            // E064: an owned field's handle is consumed while the field still
            // holds it (it was not moved out and the field cleared first), leaving
            // the field aliasing a freed pointer.
            AbsVal::Handle(h)
                if expected.kind == PointerKind::Owned
                    && h.from_field
                        .as_ref()
                        .is_some_and(|f| frame.field(f) != FieldState::Null) =>
            {
                report.push(
                    Diagnostic::error("E064", "owned handle consumed without clearing its field")
                        .with_java(Some(loc))
                        .note(
                            "move the handle out (read it to a local and set the field to `0`) \
                             *before* consuming it, so the field never aliases a freed pointer",
                        ),
                )
            }
            _ => {}
        }
    }

    // E065: a handle passed to an *exclusive* slot (`@Mut`/`@Owned`) of this call
    // is provably the same handle as another argument of the same call — a
    // mutable borrow (or a move) may not be aliased by any other borrow. Two
    // `@Ref`s of the same handle are fine. Must-alias only (shared `HandleId`),
    // so a merely-possible one-path overlap (`id == None`) never triggers it.
    if aliases_exclusive(contract, frame, base) {
        report.push(
            Diagnostic::error(
                "E065",
                "the same handle is aliased across one call's arguments",
            )
            .with_java(Some(flow_loc(class, method, code, idx)))
            .note(format!(
                "`{}` borrows it exclusively (`@Mut`/`@Owned`), so no other \
                     argument of the same call may be the same handle",
                target.name
            )),
        );
    }
}

/// True if some exclusive (`@Mut`/`@Owned`) handle argument of the call shares a
/// concrete must-alias identity with another handle argument.
fn aliases_exclusive(contract: &Contract, frame: &Frame, base: usize) -> bool {
    let arg_id = |i: usize| -> Option<&HandleId> {
        match frame.stack.get(base + i).map(|s| &s.val) {
            Some(AbsVal::Handle(h)) => h.id.as_ref(),
            _ => None,
        }
    };
    for (i, pi) in contract.params.iter().enumerate() {
        let exclusive = matches!(
            pi.as_ref().map(|p| p.kind),
            Some(PointerKind::Mut) | Some(PointerKind::Owned)
        );
        if !exclusive {
            continue;
        }
        let Some(id_i) = arg_id(i) else { continue };
        for (j, pj) in contract.params.iter().enumerate() {
            if j == i || pj.is_none() {
                continue;
            }
            if arg_id(j) == Some(id_i) {
                return true;
            }
        }
    }
    false
}

/// Check a `putfield` into an owned handle field (on `this`): storing a forged
/// value into a handle slot is E060 (it supersedes W012); overwriting a field
/// that may still hold a live handle is W012. Clearing (`= 0`) is always fine.
#[allow(clippy::too_many_arguments)]
fn check_field_store(
    class: &FlowClass,
    method: &FlowMethod,
    code: &MethodCode,
    idx: usize,
    frame: &Frame,
    field: &crate::code::MemberRef,
    owned_fields: &HashMap<String, Pointer>,
    report: &mut Report,
) {
    if !owned_fields.contains_key(&field.name) {
        return;
    }
    // Stack at a `putfield` is `[…, receiver, value]`; the value is on top.
    let argn = frame.stack.len();
    if argn < 2 || frame.stack[argn - 2].val != AbsVal::This {
        return; // not a store on `this` — conservatively untracked
    }
    let value = &frame.stack[argn - 1].val;
    let loc = flow_loc(class, method, code, idx);
    match value {
        // Arithmetic / non-handle long stored into a handle field — forged.
        AbsVal::Forged => report.push(
            Diagnostic::error("E060", "a non-handle value is stored into a handle field")
                .with_java(Some(loc))
                .note(format!(
                    "`{}` is an `@Owned` handle field; only a handle value may be stored in it \
                     (arithmetic on a handle, e.g. `{}++`, forges an invalid pointer)",
                    field.name, field.name
                )),
        ),
        // Overwriting a field that may still own a live handle leaks the old one.
        AbsVal::Handle(_) if frame.field(&field.name) != FieldState::Null => report.push(
            Diagnostic::warning("W012", "owned handle field may be overwritten while live")
                .with_java(Some(loc))
                .note(format!(
                    "`{}` may already hold a live handle here; dispose it (or prove it null) \
                     before assigning a new one",
                    field.name
                )),
        ),
        _ => {}
    }
}

/// Check an owned-returning method's `*return`: returning `this.<field>` directly
/// hands the field's handle to the caller while the field still holds it (E064),
/// and counts as a disposal path for W013.
fn check_owned_return(
    class: &FlowClass,
    method: &FlowMethod,
    code: &MethodCode,
    idx: usize,
    frame: &Frame,
    consumed: &mut BTreeSet<String>,
    report: &mut Report,
) {
    if method.ret.as_ref().map(|p| p.kind) != Some(PointerKind::Owned) {
        return;
    }
    let Some(Slot {
        val: AbsVal::Handle(h),
        ..
    }) = frame.stack.last()
    else {
        return;
    };
    let Some(f) = &h.from_field else { return };
    consumed.insert(f.clone());
    if frame.field(f) != FieldState::Null {
        report.push(
            Diagnostic::error(
                "E064",
                "owned field handle escapes via return without clearing",
            )
            .with_java(Some(flow_loc(class, method, code, idx)))
            .note(
                "the caller now owns this handle, but the field still aliases it; null the \
                     field before returning it",
            ),
        );
    }
}

/// Build a Java source location for a flow diagnostic, pinned to the method it
/// fires in (with the source file + line, when the class carries them).
fn flow_loc(class: &FlowClass, method: &FlowMethod, code: &MethodCode, idx: usize) -> JavaLoc {
    let descriptor = match (&class.source_file, code.insns.get(idx).and_then(|i| i.line)) {
        (Some(file), Some(line)) => format!("  [{file}:{line}]"),
        (Some(file), None) => format!("  [{file}]"),
        _ => method.descriptor.clone(),
    };
    JavaLoc {
        class: class.internal_name.clone(),
        method: method.name.clone(),
        descriptor,
    }
}

// ===========================================================================
// W014 — handle exposed on a public/protected surface (declaration-level)
// ===========================================================================

/// W014 — a handle annotation on a `public`/`protected` member lets a raw
/// pointer escape the class's controlled boundary, where external (or untrusted)
/// code can read, forge, or misuse it. Private / package-private members — the
/// normal `private static native` shape — are fine.
fn check_exposed_handles(class: &FlowClass, report: &mut Report) {
    for f in &class.fields {
        if (f.is_public || f.is_protected) && f.handle.is_some() {
            gated(
                report,
                &[&f.suppressed[..], &class.suppressed[..]],
                Diagnostic::warning("W014", "handle exposed on a public/protected field")
                    .with_java(Some(field_loc(class, f)))
                    .note(
                        "a raw handle reachable outside the class can be read or forged by \
                         callers; keep handle fields private",
                    ),
            );
        }
    }
    for m in &class.methods {
        if !(m.is_public || m.is_protected) {
            continue;
        }
        let supp = [&m.suppressed[..], &class.suppressed[..]];
        if m.ret.is_some() {
            gated(
                report,
                &supp,
                Diagnostic::warning("W014", "handle exposed on a public/protected method return")
                    .with_java(Some(method_loc(class, m))),
            );
        }
        if m.params.iter().any(Option::is_some) {
            gated(
                report,
                &supp,
                Diagnostic::warning(
                    "W014",
                    "handle exposed on a public/protected method parameter",
                )
                .with_java(Some(method_loc(class, m))),
            );
        }
    }
}

fn field_loc(class: &FlowClass, f: &FlowField) -> JavaLoc {
    JavaLoc {
        class: class.internal_name.clone(),
        method: f.name.clone(),
        descriptor: format!(": {}", f.descriptor),
    }
}

fn method_loc(class: &FlowClass, m: &FlowMethod) -> JavaLoc {
    JavaLoc {
        class: class.internal_name.clone(),
        method: m.name.clone(),
        descriptor: m.descriptor.clone(),
    }
}
