//! The erased UI node and its codec.
//!
//! [`NodeUi`]'s [`gantz_core::Node`] supertrait makes [`DynNode`] a
//! self-sufficient working-graph weight. One erased value serves rendering,
//! compilation and evaluation. The [`NodeCodec`] is the
//! [`gantz_edit::codec::NodeCodec`] over [`DynNode`]. An application composes
//! one from the [`kind`] of each of its node types.

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

/// One node type of the app's node set. See [`kind`].
pub type Kind = gantz_edit::codec::Kind<DynNode>;

/// A reified UI node paired with the eraser of its kind.
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

/// The [`Kind`] of the UI node type `T`.
///
/// ```ignore
/// let sugars = gantz_format::Sugars::default().with(gantz_format::CoreSugar);
/// let codec = gantz_egui::node::NodeCodec::new(
///     sugars,
///     [
///         gantz_egui::node::kind::<gantz_core::node::Expr>(),
///         gantz_egui::node::kind::<gantz_egui::node::Comment>(),
///     ],
/// );
/// ```
pub fn kind<T>() -> Kind
where
    T: NodeUi + gantz_nodetag::NodeTag + serde::Serialize + serde::de::DeserializeOwned + 'static,
{
    Kind::typed::<T>(|node| Box::new(node), as_any)
}

/// The node of `node` as `Any`, for a downcast to its concrete type.
fn as_any(node: &DynNode) -> &dyn Any {
    let node: &dyn gantz_core::Node = &**node;
    node
}
