//! The DSP node types.
//!
//! Each node is a [`gantz_nodedef::NodeDef`], whose expr is a placeholder, since
//! DSP nodes are inert in the Steel world. Its view is `gantz_ui` data.
//! [`NodeDsp`] gives the audio behaviour and
//! [`ToNodeDsp`](crate::ToNodeDsp) the discovery.
//! The `~` keyword-name prefix marks them as dsp nodes.

pub use buffer::Buffer;
pub use bus::Bus;
pub use envgen::Envgen;
pub use out::Out;
pub use pack::Pack;
pub use sample::Sample;
pub use scope_out::ScopeOut;
pub use sum::Sum;
pub use unit::{InvalidUnitNode, UnitNode};
pub use unpack::Unpack;

use crate::dsp::{DspNode, NodeDsp};
use gantz_nodedef::{DynNodeDef, NodeDef};
use std::sync::Arc;

pub mod buffer;
pub mod bus;
pub mod envgen;
pub mod out;
pub mod pack;
pub mod sample;
pub mod scope_out;
pub mod sum;
pub mod unit;
pub mod unpack;

/// The definition of the DSP node type `D`. Its decoded node is a [`DspNode`],
/// so the synthdef compiler finds the [`NodeDsp`] of any DSP node, also one
/// that another crate defines.
pub fn node_def<D: NodeDef + NodeDsp>() -> Arc<dyn DynNodeDef> {
    gantz_nodedef::of_with::<D>(|node| Arc::new(DspNode::new(node)))
}

/// The definitions of the DSP node types of this crate.
pub fn node_defs() -> Vec<Arc<dyn DynNodeDef>> {
    vec![
        node_def::<UnitNode>(),
        node_def::<Out>(),
        node_def::<ScopeOut>(),
        node_def::<Pack>(),
        node_def::<Sum>(),
        node_def::<Unpack>(),
        node_def::<Bus>(),
        node_def::<Sample>(),
        node_def::<Buffer>(),
        node_def::<Envgen>(),
    ]
}

pub(crate) fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}
