use crate::node::{self, Conns, Node};
use gantz_ca::CaHash;
use serde::{Deserialize, Serialize};
use std::fmt;
use steel::parser::lexer::TokenStream;
use thiserror::Error;

/// A node that conditionally activates different subsets of its outputs.
///
/// Like [`super::Expr`], the expression uses `$var` placeholders for inputs.
/// Unlike `Expr`, the expression **must** return `(list branch-index value(s))`
/// where `branch-index` selects which branch's outputs are activated.
///
/// Each branch is a [`Conns`] bitmask specifying which outputs are active when
/// that branch is selected. The number of outputs is inferred from the `Conns`
/// length (all branches must have the same length).
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, CaHash)]
#[cahash("gantz.branch")]
pub struct Branch {
    src: String,
    branches: Vec<Conns>,
    /// Unique `$` variable names in order of first appearance (cached).
    /// Skipped during serialization and recomputed on deserialization.
    #[serde(skip)]
    #[cahash(skip)]
    vars: Vec<String>,
}

impl<'de> Deserialize<'de> for Branch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct BranchData {
            src: String,
            branches: Vec<Conns>,
        }
        let data = BranchData::deserialize(deserializer)?;
        if data.branches.is_empty() {
            return Err(serde::de::Error::custom(
                "branches must have at least 1 entry",
            ));
        }
        let out_len = data.branches[0].len();
        if out_len < 1 || out_len > 16 {
            return Err(serde::de::Error::custom(format!(
                "output count must be in 1..=16, got {out_len}",
            )));
        }
        for (i, conns) in data.branches.iter().enumerate().skip(1) {
            if conns.len() != out_len {
                return Err(serde::de::Error::custom(format!(
                    "branch {i} has {} outputs but branch 0 has {out_len}",
                    conns.len(),
                )));
            }
        }
        let vars = super::expr::vars_from_src(&data.src);
        Ok(Branch {
            src: data.src,
            branches: data.branches,
            vars,
        })
    }
}

/// An error occurred while constructing the `Branch` node.
#[derive(Debug, Error)]
pub enum BranchNewError {
    /// Failed to parse a valid expression.
    #[error("failed to parse a valid expr: {err}")]
    InvalidExpr {
        #[from]
        err: steel::rerrs::SteelErr,
    },
    /// The parsed result contained no expression.
    #[error("parsed result contains no expression")]
    Empty,
    /// No branches provided.
    #[error("branches must have at least 1 entry, got 0")]
    NoBranches,
    /// Output count (Conns length) is out of range.
    #[error("output count must be in 1..=16, got {0}")]
    InvalidOutputs(usize),
    /// Branches have inconsistent Conns lengths.
    #[error("branch {ix} has {actual} outputs but branch 0 has {expected}")]
    ConnsLenMismatch {
        ix: usize,
        expected: usize,
        actual: usize,
    },
}

impl Branch {
    /// Construct a `Branch` node.
    ///
    /// - `src` is a Steel expression that must return `(list branch-index value(s))`.
    /// - `branches` defines the output activation mask for each branch. All
    ///   entries must have the same `Conns` length (1-16), which determines the
    ///   number of outputs.
    ///
    /// Returns an `Err` if validation fails or the expression cannot be parsed.
    pub fn new(src: impl Into<String>, branches: Vec<Conns>) -> Result<Self, BranchNewError> {
        let src: String = src.into();
        if branches.is_empty() {
            return Err(BranchNewError::NoBranches);
        }
        let out_len = branches[0].len();
        if out_len < 1 || out_len > 16 {
            return Err(BranchNewError::InvalidOutputs(out_len));
        }
        for (ix, conns) in branches.iter().enumerate().skip(1) {
            if conns.len() != out_len {
                return Err(BranchNewError::ConnsLenMismatch {
                    ix,
                    expected: out_len,
                    actual: conns.len(),
                });
            }
        }
        let vars = super::expr::vars_from_src(&src);
        // Validate that the source parses successfully.
        let exprs = steel::steel_vm::engine::Engine::emit_ast(&src)?;
        if exprs.is_empty() {
            return Err(BranchNewError::Empty);
        }
        Ok(Branch {
            src,
            branches,
            vars,
        })
    }

    /// The source string that was used to create this node.
    pub fn src(&self) -> &str {
        &self.src
    }

    /// The number of outputs, inferred from the `Conns` length.
    pub fn outputs(&self) -> u8 {
        self.branches[0].len() as u8
    }

    /// The branch output activation masks.
    pub fn branch_conns(&self) -> &[Conns] {
        &self.branches
    }

    /// The number of branches.
    pub fn n_branches(&self) -> usize {
        self.branches.len()
    }

    /// Set the number of outputs, resizing all branch `Conns` accordingly.
    ///
    /// Existing bits are preserved up to `min(old, new)` length. New bits
    /// default to `false`.
    ///
    /// # Panics
    ///
    /// Panics if `n` is 0 or greater than 16.
    pub fn set_outputs(&mut self, n: u8) {
        assert!(n >= 1 && n <= 16, "outputs must be in 1..=16, got {n}");
        let new_len = n as usize;
        for conns in &mut self.branches {
            *conns = resize_conns(*conns, new_len);
        }
    }

    /// Replace the branch definitions.
    ///
    /// All `Conns` must have the same length, matching the current output count.
    ///
    /// # Panics
    ///
    /// Panics if `branches` is empty or any `Conns` length mismatches.
    pub fn set_branch_conns(&mut self, branches: Vec<Conns>) {
        assert!(!branches.is_empty(), "need at least 1 branch");
        let out_len = self.outputs() as usize;
        for (i, c) in branches.iter().enumerate() {
            assert_eq!(
                c.len(),
                out_len,
                "branch {i} has {} outputs but expected {out_len}",
                c.len(),
            );
        }
        self.branches = branches;
    }
}

/// Resize a `Conns` to a new length, preserving existing bits.
fn resize_conns(old: Conns, new_len: usize) -> Conns {
    let mut new = Conns::unconnected(new_len).expect("new_len out of range");
    let copy_len = old.len().min(new_len);
    for i in 0..copy_len {
        if let Some(true) = old.get(i) {
            new.set(i, true).unwrap();
        }
    }
    new
}

impl Default for Branch {
    /// A default 2-output if/else branch.
    fn default() -> Self {
        Branch::new(
            "(if (= 0 $x) (list 0 '()) (list 1 '()))",
            vec![
                Conns::try_from([true, false]).unwrap(),
                Conns::try_from([false, true]).unwrap(),
            ],
        )
        .unwrap()
    }
}

impl Node for Branch {
    fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
        self.vars.len()
    }

    fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
        self.outputs() as usize
    }

    fn branches(&self, _ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        self.branches
            .iter()
            .map(|conns| node::EvalConf::Set(*conns))
            .collect()
    }

    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        let tts = TokenStream::new(&self.src, true, None);
        let new_src = super::expr::interpolate_tokens(tts, &self.vars, ctx.inputs());
        let exprs = steel::steel_vm::engine::Engine::emit_ast(&new_src)
            .map_err(|e| node::ExprError::custom(e))?;
        if exprs.len() == 1 {
            Ok(exprs.into_iter().next().unwrap())
        } else {
            let exprs = exprs
                .iter()
                .map(|expr| format!("{expr}"))
                .collect::<Vec<_>>()
                .join(" ");
            let out_src = format!("(begin {})", exprs);
            node::parse_expr(&out_src)
        }
    }

    /// Only generate the state binding if the expr references `state`.
    fn stateful(&self, _ctx: node::MetaCtx) -> bool {
        self.src().contains("state")
    }

    /// Registers a state slot just in case `state` is referenced by the expr.
    fn register(&self, ctx: node::RegCtx<'_, '_>) {
        let (_, path, vm) = ctx.into_parts();
        node::state::init_value_if_absent(vm, path, || steel::SteelVal::Void).unwrap();
    }
}

impl fmt::Display for Branch {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.src)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_branch_conns() -> Vec<Conns> {
        vec![
            Conns::try_from([true, false]).unwrap(),
            Conns::try_from([false, true]).unwrap(),
        ]
    }

    #[test]
    fn test_new_valid() {
        let b = Branch::new(
            "(if (equal? 0 $x) (list 0 '()) (list 1 '()))",
            two_branch_conns(),
        )
        .unwrap();
        assert_eq!(b.outputs(), 2);
        assert_eq!(b.n_branches(), 2);
        assert_eq!(b.src(), "(if (equal? 0 $x) (list 0 '()) (list 1 '()))");
    }

    #[test]
    fn test_new_single_branch() {
        let b = Branch::new("(list 0 $x)", vec![Conns::try_from([true]).unwrap()]).unwrap();
        assert_eq!(b.outputs(), 1);
        assert_eq!(b.n_branches(), 1);
    }

    #[test]
    fn test_new_extracts_vars() {
        let b = Branch::new(
            "(if (equal? 0 $a) (list 0 $b) (list 1 $c))",
            two_branch_conns(),
        )
        .unwrap();
        let ctx = node::MetaCtx::new(&|_| None);
        assert_eq!(b.n_inputs(ctx), 3);
    }

    #[test]
    fn test_new_no_branches() {
        let err = Branch::new("(list 0 '())", vec![]).unwrap_err();
        assert!(matches!(err, BranchNewError::NoBranches));
    }

    #[test]
    fn test_new_invalid_outputs_zero() {
        let err = Branch::new("(list 0 '())", vec![Conns::unconnected(0).unwrap()]).unwrap_err();
        assert!(matches!(err, BranchNewError::InvalidOutputs(0)));
    }

    #[test]
    fn test_new_invalid_outputs_too_high() {
        let err = Branch::new("(list 0 '())", vec![Conns::unconnected(17).unwrap()]).unwrap_err();
        assert!(matches!(err, BranchNewError::InvalidOutputs(17)));
    }

    #[test]
    fn test_new_conns_len_mismatch() {
        let bad = vec![
            Conns::try_from([true, false, false]).unwrap(),
            Conns::try_from([false, true]).unwrap(),
        ];
        let err = Branch::new("(list 0 '())", bad).unwrap_err();
        assert!(matches!(
            err,
            BranchNewError::ConnsLenMismatch {
                ix: 1,
                expected: 3,
                actual: 2,
            }
        ));
    }

    #[test]
    fn test_new_invalid_expr() {
        let err = Branch::new("(((", two_branch_conns()).unwrap_err();
        assert!(matches!(err, BranchNewError::InvalidExpr { .. }));
    }

    #[test]
    fn test_node_trait_branches() {
        let b = Branch::new(
            "(if (equal? 0 $x) (list 0 '()) (list 1 '()))",
            two_branch_conns(),
        )
        .unwrap();
        let ctx = node::MetaCtx::new(&|_| None);
        let branches = b.branches(ctx);
        assert_eq!(branches.len(), 2);
        assert_eq!(
            branches[0],
            node::EvalConf::Set(Conns::try_from([true, false]).unwrap())
        );
        assert_eq!(
            branches[1],
            node::EvalConf::Set(Conns::try_from([false, true]).unwrap())
        );
    }

    #[test]
    fn test_set_outputs_resize() {
        let mut b = Branch::new(
            "(if (equal? 0 $x) (list 0 '()) (list 1 '()))",
            two_branch_conns(),
        )
        .unwrap();
        // Grow from 2 to 3 outputs.
        b.set_outputs(3);
        assert_eq!(b.outputs(), 3);
        for conns in b.branch_conns() {
            assert_eq!(conns.len(), 3);
        }
        // First branch: [true, false] -> [true, false, false]
        assert_eq!(b.branch_conns()[0].get(0), Some(true));
        assert_eq!(b.branch_conns()[0].get(1), Some(false));
        assert_eq!(b.branch_conns()[0].get(2), Some(false));
        // Shrink to 1.
        b.set_outputs(1);
        assert_eq!(b.outputs(), 1);
        assert_eq!(b.branch_conns()[0].get(0), Some(true));
        assert_eq!(b.branch_conns()[0].len(), 1);
    }

    #[test]
    fn test_stateful() {
        let b = Branch::new("(begin (set! state $x) (list 0 state))", two_branch_conns()).unwrap();
        let ctx = node::MetaCtx::new(&|_| None);
        assert!(b.stateful(ctx));

        let b2 = Branch::new(
            "(if (equal? 0 $x) (list 0 '()) (list 1 '()))",
            two_branch_conns(),
        )
        .unwrap();
        assert!(!b2.stateful(ctx));
    }

    #[test]
    fn test_display() {
        let b = Branch::new("(list 0 '())", two_branch_conns()).unwrap();
        assert_eq!(format!("{b}"), "(list 0 '())");
    }

    #[test]
    fn test_serde_roundtrip() {
        let original = Branch::new(
            "(if (equal? 0 $x) (list 0 '()) (list 1 '()))",
            two_branch_conns(),
        )
        .unwrap();
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Branch = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_deser_no_branches() {
        let json = r#"{"src":"(list 0 '())","branches":[]}"#;
        assert!(serde_json::from_str::<Branch>(json).is_err());
    }

    #[test]
    fn test_deser_conns_len_mismatch() {
        let json = r#"{"src":"(list 0 '())","branches":["100","01"]}"#;
        assert!(serde_json::from_str::<Branch>(json).is_err());
    }

    #[test]
    fn test_optional_input() {
        let b = Branch::new(
            "(if (Some? $?x) (list 0 (Some->value $?x)) (list 1 '()))",
            two_branch_conns(),
        )
        .unwrap();
        let ctx = node::MetaCtx::new(&|_| None);
        assert_eq!(b.n_inputs(ctx), 1);

        // Verify expr with unconnected optional input produces (None).
        let outputs = Conns::try_from([true, false]).unwrap();
        let expr_ctx = node::ExprCtx::new(&|_| None, &[0], &[None], &outputs);
        let expr = b.expr(expr_ctx).unwrap();
        let s = format!("{expr}");
        assert!(s.contains("(None)"), "expected (None) in expr: {s}");
    }
}
