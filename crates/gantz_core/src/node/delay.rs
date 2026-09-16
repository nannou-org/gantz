//! The unit-[`Delay`] node. Pd-style cross-evaluation feedback.

use crate::node::{self, Node};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// A unit delay. It outputs the value its input received on the previous
/// evaluation.
///
/// The compiler treats delays as intrinsics and generates no node fn. The
/// stored value is bound when an evaluation begins. The input is written to
/// state at the point it is produced. Evaluation never propagates through a
/// delay. A feedback cycle is legal exactly when it passes through one. The
/// value crosses between evaluations rather than looping within one.
///
/// Before the first write the stored value is `'()`. Downstream nodes must
/// guard for this, for example `(if (number? $x) $x 0)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, NodeTag)]
pub struct Delay;

impl Node for Delay {
    /// Never called during compilation. The compiler special-cases delays.
    /// See [`Delay`].
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
