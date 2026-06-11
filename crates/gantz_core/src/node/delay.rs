//! The unit-[`Delay`] node: pd-style cross-evaluation feedback.

use crate::node::{self, Node};
use gantz_ca::CaHash;
use serde::{Deserialize, Serialize};

/// A unit delay: outputs the value its input received on the *previous*
/// evaluation.
///
/// The compiler treats delays as intrinsics (no node fn is generated): the
/// stored value is bound when an evaluation begins, and the input is written
/// to state at the point it is produced. Evaluation never propagates
/// *through* a delay, so a feedback cycle is legal exactly when it passes
/// through one - the value crosses between evaluations rather than looping
/// within one.
///
/// Before the first write the stored value is `'()`; downstream nodes guard
/// accordingly (e.g. `(if (number? $x) $x 0)`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, CaHash)]
#[cahash("gantz.delay")]
pub struct Delay;

impl Node for Delay {
    /// Never called during compilation - delays are special-cased:
    /// no node fn is generated; the read is a state lookup bound at the top
    /// of the evaluation and the write a state insert where the input value
    /// is produced.
    fn expr(&self, _ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        node::parse_expr("'()")
    }

    fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
        1
    }

    fn delay(&self, _ctx: node::MetaCtx) -> bool {
        true
    }

    /// The delay holds its previous-evaluation value in state.
    fn stateful(&self, _ctx: node::MetaCtx) -> bool {
        true
    }

    fn register(&self, ctx: node::RegCtx<'_, '_>) {
        let (_, path, vm) = ctx.into_parts();
        node::state::init_value_if_absent(vm, path, || steel::SteelVal::ListV(Default::default()))
            .unwrap();
    }
}
