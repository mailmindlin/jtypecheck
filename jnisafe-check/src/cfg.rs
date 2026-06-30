//! A lightweight control-flow view over a method's instructions.
//!
//! The flow interpreter works at *instruction* granularity rather than basic
//! blocks — exception edges can leave any instruction inside a `try`, so a
//! per-instruction successor relation keeps that simple. This module just
//! answers "where can control go from instruction `i`?" (normal and
//! exceptional) and "does the method exit here?"; the dataflow itself lives in
//! [`flow`](crate::flow).

use crate::code::{MethodCode, Op};

/// Per-instruction control-flow over a [`MethodCode`].
pub struct Cfg<'c> {
    code: &'c MethodCode,
    /// `(start_offset, end_offset, handler_insn_index)` per exception range.
    handlers: Vec<(u32, u32, usize)>,
}

impl<'c> Cfg<'c> {
    pub fn new(code: &'c MethodCode) -> Self {
        let handlers = code
            .exceptions
            .iter()
            .filter_map(|e| code.index_of(e.handler).map(|h| (e.start, e.end, h)))
            .collect();
        Cfg { code, handlers }
    }

    /// Normal-flow successor instruction indices of instruction `idx`.
    pub fn normal_succ(&self, idx: usize) -> Vec<usize> {
        let next = idx + 1;
        let has_next = next < self.code.insns.len();
        match &self.code.insns[idx].op {
            // Leave the method (or transfer only via an exception edge).
            Op::Return { .. } | Op::Athrow => Vec::new(),
            Op::Goto(t) => self.code.index_of(*t).into_iter().collect(),
            Op::Branch { target, .. } => {
                let mut v = Vec::new();
                if has_next {
                    v.push(next);
                }
                if let Some(t) = self.code.index_of(*target) {
                    v.push(t);
                }
                v
            }
            Op::Switch { targets } => {
                let mut v: Vec<usize> = targets
                    .iter()
                    .filter_map(|t| self.code.index_of(*t))
                    .collect();
                v.sort_unstable();
                v.dedup();
                v
            }
            _ if has_next => vec![next],
            _ => Vec::new(),
        }
    }

    /// Exception-handler successor indices for instruction `idx` — handlers whose
    /// `try` range covers it (an exception may be thrown at any covered insn).
    pub fn exc_succ(&self, idx: usize) -> Vec<usize> {
        let off = self.code.insns[idx].offset;
        let mut v: Vec<usize> = self
            .handlers
            .iter()
            .filter(|(start, end, _)| off >= *start && off < *end)
            .map(|(_, _, h)| *h)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// `true` if control leaves the method at `idx`: a `return`, or an `athrow`
    /// not caught by any handler covering it.
    pub fn is_exit(&self, idx: usize) -> bool {
        match &self.code.insns[idx].op {
            Op::Return { .. } => true,
            Op::Athrow => self.exc_succ(idx).is_empty(),
            _ => false,
        }
    }
}
