//! The erased UI node and the value-level codec between typed nodes and the
//! registry's [`NodeData`] representation.
//!
//! [`NodeUi`]'s [`gantz_core::Node`] supertrait makes [`DynNode`] a
//! self-sufficient working-graph weight. One erased value serves rendering,
//! compilation and evaluation. The [`NodeCodec`] carries the node set as
//! values rather than as a `Box<dyn Trait>` serde impl. An application
//! composes one with [`crate::ui_node_codec!`]. Its type list is the app's
//! node-set manifest. A node type is storable exactly when it is listed
//! there.

use crate::NodeUi;
use gantz_ca::{ContentAddr, DataGraph, NodeData};
use gantz_core::Builtins;
use gantz_core::data::{EraseNodeError, ReifyError, ReifyNodeError};
use gantz_core::node::graph::Graph;
use petgraph::visit::EdgeRef;
use std::any::Any;
use std::collections::HashMap;

/// A UI-capable node erased to a trait object.
///
/// Via [`NodeUi`]'s supertrait, `DynNode` implements [`gantz_core::Node`]
/// as well as `NodeUi`. It slots directly into [`Graph`] weights,
/// compilation and the widgets.
pub type DynNode = Box<dyn NodeUi>;

/// A reified node paired with the monomorphic eraser for its concrete type.
///
/// Produced by [`NodeCodec::reify_ui`]. The codec arm that decoded the node
/// knows its concrete type, so it captures an eraser that downcasts and runs
/// [`gantz_core::data::erase_node_typed`]. No trait-object serde is
/// involved.
pub struct NodeUiInstance {
    /// The reified node.
    pub node: DynNode,
    erase: fn(&dyn Any) -> Result<NodeData, EraseNodeError>,
}

/// A value-level codec between a node set's typed nodes and their stored
/// [`NodeData`] form.
///
/// Two plain function pointers, so a codec is `Copy` and can be composed as
/// a `const`. Build one with [`crate::ui_node_codec!`].
#[derive(Clone, Copy)]
pub struct NodeCodec {
    reify: fn(&NodeData) -> Result<NodeUiInstance, ReifyNodeError>,
    sugars: fn() -> gantz_format::Sugars<'static>,
}

/// The reified builtin palette. One typed [`DynNode`] instance per builtin,
/// keyed by its erased content address.
///
/// The stored instances serve compilation through the [`gantz_core::Node`]
/// supertrait upcast. They also serve [`NodeUi`] introspection such as
/// palette docs and socket previews. No fresh instances are minted per
/// frame.
#[derive(Default)]
pub struct UiBuiltins {
    map: HashMap<ContentAddr, DynNode>,
}

/// Failure to normalize a node's data form through its type. The reify or
/// the re-erasure failed.
#[derive(Clone, Debug, thiserror::Error)]
pub enum NormalizeNodeError {
    /// The stored form failed to decode. The tag is unknown or the fields
    /// are invalid.
    #[error(transparent)]
    Reify(#[from] ReifyNodeError),
    /// The reified node failed to erase back to data.
    #[error(transparent)]
    Erase(#[from] EraseNodeError),
}

// Lets reference-transparent passes such as the DSP compiler's flattening
// find the underlying `Ref` within an erased UI node. `FnNamedRef`
// deliberately does not match. A function value references a graph without
// standing in for it.
impl gantz_core::node::AsRefNode for DynNode {
    fn as_ref_node(&self) -> Option<&gantz_core::node::Ref> {
        let node: &dyn gantz_core::Node = &**self;
        let any: &dyn Any = node;
        any.downcast_ref::<crate::node::NamedRef>()
            .map(|nr| nr.ref_())
            .or_else(|| any.downcast_ref::<gantz_core::node::Ref>())
    }
}

impl NodeUiInstance {
    /// Pair a reified node with its concrete type's eraser.
    ///
    /// `erase` receives the node upcast to `&dyn Any` and must downcast to
    /// the node's own concrete type. The codec arm is responsible for keeping
    /// the two in agreement. See [`crate::ui_node_codec!`].
    pub fn new(node: DynNode, erase: fn(&dyn Any) -> Result<NodeData, EraseNodeError>) -> Self {
        Self { node, erase }
    }

    /// Erase the node back to its canonical data form. See
    /// [`gantz_core::data::erase_node_typed`].
    pub fn erase(&self) -> Result<NodeData, EraseNodeError> {
        let node: &dyn gantz_core::Node = &*self.node;
        let any: &dyn Any = node;
        (self.erase)(any)
    }
}

impl UiBuiltins {
    /// Reify each builtin once through the codec.
    ///
    /// Failures are returned for logging. A builtin that fails to reify, for
    /// example on a tag missing from the codec, degrades to a lookup miss.
    pub fn reify(builtins: &Builtins, codec: &NodeCodec) -> (Self, Vec<ReifyNodeError>) {
        let mut map = HashMap::new();
        let mut errs = vec![];
        for name in builtins.names() {
            let node_data = builtins.node_data(name).expect("named builtin");
            match codec.reify_ui(node_data) {
                Ok(inst) => {
                    map.insert(node_data.content_addr(), inst.node);
                }
                Err(e) => errs.push(e),
            }
        }
        (Self { map }, errs)
    }

    /// The reified instance of the builtin with the given content address.
    pub fn get(&self, ca: &ContentAddr) -> Option<&DynNode> {
        self.map.get(ca)
    }
}

impl NodeCodec {
    /// Compose a codec from its reify dispatch and its node set's sugar
    /// source. See [`crate::ui_node_codec!`] for the standard construction.
    pub const fn new(
        reify: fn(&NodeData) -> Result<NodeUiInstance, ReifyNodeError>,
        sugars: fn() -> gantz_format::Sugars<'static>,
    ) -> Self {
        Self { reify, sugars }
    }

    /// Reify one stored node to a typed [`NodeUiInstance`].
    pub fn reify_ui(&self, node_data: &NodeData) -> Result<NodeUiInstance, ReifyNodeError> {
        (self.reify)(node_data)
    }

    /// Reify a stored graph. Node weights go through [`Self::reify_ui`].
    /// Indices and edges are preserved verbatim. This mirrors
    /// [`gantz_core::data::reify`].
    pub fn reify_graph(&self, dg: &DataGraph) -> Result<Graph<DynNode>, ReifyError> {
        let mut out = Graph::with_capacity(dg.node_count(), dg.edge_count());
        for (node_ix, node_data) in dg.node_weights().enumerate() {
            let inst = self
                .reify_ui(node_data)
                .map_err(|source| ReifyError { node_ix, source })?;
            out.add_node(inst.node);
        }
        for e in dg.edge_references() {
            out.add_edge(e.source(), e.target(), *e.weight());
        }
        Ok(out)
    }

    /// Round-trip a node's data form through its type. Reify, then erase.
    ///
    /// This validates the fields against the node's own serde. It recomputes
    /// the canonical form and the refs and blobs columns from the node's
    /// reporting.
    pub fn normalize(&self, node_data: &NodeData) -> Result<NodeData, NormalizeNodeError> {
        Ok(self.reify_ui(node_data)?.erase()?)
    }

    /// The node set's composed `.gantz` keyword sugar.
    pub fn sugars(&self) -> gantz_format::Sugars<'static> {
        (self.sugars)()
    }
}

/// Compose a [`crate::node::NodeCodec`] over a node set.
///
/// Takes the node set's `gantz_format::NodeSugar` carrier type and the list
/// of node types. The list is the application's node-set manifest. A node
/// type is storable exactly when it is listed here. Its own `NodeTag` and
/// serde fix its wire tag, shape and content address. Each listed type must
/// implement `gantz_nodetag::NodeTag`, `serde::Serialize`,
/// `serde::de::DeserializeOwned` and [`crate::NodeUi`]. The calling crate
/// must depend on `serde`.
///
/// Reifying data whose tag is not listed fails with a
/// `gantz_core::data::ReifyNodeError` naming the tag.
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
    ($carrier:ty { $($ty:ty),+ $(,)? }) => {{
        fn reify(
            node_data: &$crate::gantz_ca::NodeData,
        ) -> ::std::result::Result<
            $crate::node::NodeUiInstance,
            $crate::gantz_core::data::ReifyNodeError,
        > {
            $(
                if node_data.tag == <$ty as $crate::gantz_nodetag::NodeTag>::TAG {
                    return $crate::gantz_core::data::reify_node_concrete::<$ty>(node_data)
                        .map(|node| $crate::node::NodeUiInstance::new(
                            ::std::boxed::Box::new(node),
                            |any: &dyn ::std::any::Any| {
                                let node = any
                                    .downcast_ref::<$ty>()
                                    .expect("tag-matched type");
                                $crate::gantz_core::data::erase_node_typed(node)
                            },
                        ));
                }
            )+
            ::std::result::Result::Err($crate::gantz_core::data::ReifyNodeError {
                tag: node_data.tag.clone(),
                source: <$crate::gantz_ca::DatumError as ::serde::de::Error>::custom(
                    "unknown node type tag: not listed in `ui_node_codec!`",
                ),
            })
        }
        $crate::node::NodeCodec::new(
            reify,
            <$carrier as $crate::gantz_format::NodeSugar>::sugar,
        )
    }};
}
