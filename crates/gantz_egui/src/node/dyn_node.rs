//! The erased UI node and its codec.
//!
//! [`NodeUi`]'s [`gantz_core::Node`] supertrait makes [`DynNode`] a
//! self-sufficient working-graph weight. One erased value serves rendering,
//! compilation and evaluation. The [`NodeCodec`] is the
//! [`gantz_edit::codec::NodeCodec`] over [`DynNode`]. An application composes
//! one with [`crate::ui_node_codec!`].

use crate::NodeUi;
use std::any::Any;

pub use gantz_edit::codec::NormalizeNodeError;

/// A UI-capable node erased to a trait object.
///
/// Via [`NodeUi`]'s supertrait, `DynNode` implements [`gantz_core::Node`]
/// as well as `NodeUi`. It slots directly into graph weights, compilation
/// and the widgets.
pub type DynNode = Box<dyn NodeUi>;

/// The codec between the app's node set and its stored form.
pub type NodeCodec = gantz_edit::codec::NodeCodec<DynNode>;

/// A reified UI node paired with its concrete type's eraser.
pub type NodeUiInstance = gantz_edit::codec::NodeInstance<DynNode>;

/// The reified builtin palette. One [`DynNode`] instance per builtin.
///
/// The stored instances serve compilation through the [`gantz_core::Node`]
/// supertrait upcast. They also serve [`NodeUi`] introspection such as
/// palette docs and socket previews. No fresh instances are minted per
/// frame.
pub type UiBuiltins = gantz_edit::codec::ReifiedBuiltins<DynNode>;

// Lets reference-transparent passes such as the DSP compiler's flattening
// find the underlying `Ref` within an erased UI node. `FnNamedRef`
// deliberately does not match. A function value references a graph without
// standing in for it.
impl gantz_core::node::AsRefNode for DynNode {
    fn as_ref_node(&self) -> Option<&gantz_core::node::Ref> {
        let node: &dyn gantz_core::Node = &**self;
        let any: &dyn Any = node;
        any.downcast_ref::<gantz_edit::NamedRef>()
            .map(|nr| nr.ref_())
            .or_else(|| any.downcast_ref::<gantz_core::node::Ref>())
    }
}

/// Compose a [`crate::node::NodeCodec`] over a node set.
///
/// Takes the node set's `gantz_format::NodeSugar` carrier type and the list
/// of node types. Each listed type must implement [`crate::NodeUi`] as well.
/// See [`gantz_edit::node_codec!`].
///
/// ```ignore
/// pub struct NodeSet;
///
/// impl gantz_format::NodeSugar for NodeSet {
///     fn sugar() -> gantz_format::Sugars<'static> {
///         gantz_format::Sugars(vec![&gantz_format::CoreSugar])
///     }
/// }
///
/// pub fn codec() -> gantz_egui::node::NodeCodec {
///     gantz_egui::ui_node_codec! {
///         NodeSet {
///             gantz_core::node::Expr,
///             gantz_egui::node::Comment,
///             // ...
///         }
///     }
/// }
/// ```
#[macro_export]
macro_rules! ui_node_codec {
    ($carrier:ty { $($ty:ty),+ $(,)? }) => {
        $crate::gantz_edit::node_codec! {
            $crate::node::DynNode, |node| {
                let node: &dyn $crate::gantz_core::Node = &**node;
                node as &dyn ::std::any::Any
            };
            $carrier { $($ty),+ }
        }
    };
}
