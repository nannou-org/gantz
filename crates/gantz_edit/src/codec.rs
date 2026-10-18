//! The value-level codec between a node set's typed nodes and the registry's
//! [`NodeData`] representation.
//!
//! The [`NodeCodec`] carries the node set as values rather than as a
//! `Box<dyn Trait>` serde impl. An application composes one with
//! [`crate::node_codec!`]. Its type list is the app's node-set manifest. A
//! node type is storable exactly when it is listed there.
//!
//! A codec is generic over the reified node type `N`, such as a boxed trait
//! object. The text format and the export paths need no typed node at all.
//! They take the codec's N-free [`DataCodec`].

use gantz_ca::{ContentAddr, DataGraph, NodeData};
use gantz_core::Builtins;
use gantz_core::data::{EraseNodeError, ReifyError, ReifyNodeError};
use gantz_core::node::graph::Graph;
use std::any::Any;
use std::collections::HashMap;

/// A reified node paired with the monomorphic eraser for its concrete type.
///
/// Produced by [`NodeCodec::reify`]. The codec arm that decoded the node
/// knows its concrete type, so it captures an eraser that downcasts and runs
/// [`gantz_core::data::erase_node_typed`]. No trait-object serde is
/// involved.
pub struct NodeInstance<N> {
    /// The reified node.
    pub node: N,
    erase: fn(&N) -> Result<NodeData, EraseNodeError>,
}

/// The node-type-free part of a [`NodeCodec`]: normalize a node's data form
/// through its type, and the node set's `.gantz` keyword sugar.
///
/// Two plain function pointers, so a codec is `Copy`. Build one with
/// [`crate::node_codec!`] and take it with [`NodeCodec::data`].
#[derive(Clone, Copy)]
pub struct DataCodec {
    normalize: fn(&NodeData) -> Result<NodeData, NormalizeNodeError>,
    sugars: fn() -> gantz_format::Sugars<'static>,
}

/// A value-level codec between a node set's typed nodes `N` and their stored
/// [`NodeData`] form.
///
/// Plain function pointers, so a codec is `Copy` and can be composed as a
/// `const`. Build one with [`crate::node_codec!`].
pub struct NodeCodec<N> {
    reify: fn(&NodeData) -> Result<NodeInstance<N>, ReifyNodeError>,
    data: DataCodec,
}

/// The reified builtin palette. One typed instance per builtin, keyed by its
/// erased content address.
pub struct ReifiedBuiltins<N> {
    map: HashMap<ContentAddr, N>,
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

impl<N> NodeInstance<N> {
    /// Pair a reified node with its concrete type's eraser.
    ///
    /// `erase` must erase the node as its own concrete type. The codec arm
    /// is responsible for keeping the two in agreement. See
    /// [`crate::node_codec!`].
    pub fn new(node: N, erase: fn(&N) -> Result<NodeData, EraseNodeError>) -> Self {
        Self { node, erase }
    }

    /// Erase the node back to its canonical data form. See
    /// [`gantz_core::data::erase_node_typed`].
    pub fn erase(&self) -> Result<NodeData, EraseNodeError> {
        (self.erase)(&self.node)
    }
}

impl DataCodec {
    /// Compose a data codec from its normalize fn and its node set's sugar
    /// source. See [`crate::node_codec!`] for the standard construction.
    pub const fn new(
        normalize: fn(&NodeData) -> Result<NodeData, NormalizeNodeError>,
        sugars: fn() -> gantz_format::Sugars<'static>,
    ) -> Self {
        Self { normalize, sugars }
    }

    /// Round-trip a node's data form through its type. Reify, then erase.
    ///
    /// This validates the fields against the node's own serde. It recomputes
    /// the canonical form and the refs and blobs columns from the node's
    /// reporting.
    pub fn normalize(&self, node_data: &NodeData) -> Result<NodeData, NormalizeNodeError> {
        (self.normalize)(node_data)
    }

    /// The node set's composed `.gantz` keyword sugar.
    pub fn sugars(&self) -> gantz_format::Sugars<'static> {
        (self.sugars)()
    }
}

impl<N> NodeCodec<N> {
    /// Compose a codec from its reify dispatch and its data codec. See
    /// [`crate::node_codec!`] for the standard construction.
    pub const fn new(
        reify: fn(&NodeData) -> Result<NodeInstance<N>, ReifyNodeError>,
        data: DataCodec,
    ) -> Self {
        Self { reify, data }
    }

    /// Reify one stored node to a typed [`NodeInstance`].
    pub fn reify(&self, node_data: &NodeData) -> Result<NodeInstance<N>, ReifyNodeError> {
        (self.reify)(node_data)
    }

    /// Reify a stored graph. Node weights go through [`Self::reify`].
    /// Indices and edges are preserved verbatim.
    pub fn reify_graph(&self, dg: &DataGraph) -> Result<Graph<N>, ReifyError> {
        gantz_core::data::reify_with(dg, |nd| self.reify(nd).map(|inst| inst.node))
    }

    /// The codec's node-type-free part.
    pub fn data(&self) -> &DataCodec {
        &self.data
    }

    /// See [`DataCodec::normalize`].
    pub fn normalize(&self, node_data: &NodeData) -> Result<NodeData, NormalizeNodeError> {
        self.data.normalize(node_data)
    }

    /// See [`DataCodec::sugars`].
    pub fn sugars(&self) -> gantz_format::Sugars<'static> {
        self.data.sugars()
    }
}

impl<N> ReifiedBuiltins<N> {
    /// Reify each builtin once through the codec.
    ///
    /// Failures are returned for logging. A builtin that fails to reify, for
    /// example on a tag missing from the codec, degrades to a lookup miss.
    pub fn reify(builtins: &Builtins, codec: &NodeCodec<N>) -> (Self, Vec<ReifyNodeError>) {
        let mut map = HashMap::new();
        let mut errs = vec![];
        for name in builtins.names() {
            let node_data = builtins.node_data(name).expect("named builtin");
            match codec.reify(node_data) {
                Ok(inst) => {
                    map.insert(node_data.content_addr(), inst.node);
                }
                Err(e) => errs.push(e),
            }
        }
        (Self { map }, errs)
    }

    /// The reified instance of the builtin with the given content address.
    pub fn get(&self, ca: &ContentAddr) -> Option<&N> {
        self.map.get(ca)
    }
}

impl<N> Clone for NodeCodec<N> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<N> Copy for NodeCodec<N> {}

impl<N> Default for ReifiedBuiltins<N> {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

/// Erase `node` as the concrete type `T`. The eraser that
/// [`crate::node_codec!`] pairs with each reified node.
///
/// Panics if `node` is not a `T`. The codec arm that reified the node as a
/// `T` upholds this.
#[doc(hidden)]
pub fn erase_as<T>(node: &dyn Any) -> Result<NodeData, EraseNodeError>
where
    T: gantz_nodetag::NodeTag + serde::Serialize + gantz_core::Node + 'static,
{
    let node = node.downcast_ref::<T>().expect("tag-matched type");
    gantz_core::data::erase_node_typed(node)
}

/// Compose a [`crate::codec::NodeCodec`] over a node set.
///
/// Takes the reified node type, the node set's `gantz_format::NodeSugar`
/// carrier type and the list of node types. The node type must be a box of
/// a trait object that each listed type coerces to, and that upcasts to
/// `&dyn Any` through `$as_any`. The list is the application's node-set
/// manifest. A node type is storable exactly when it is listed here. Its own
/// `NodeTag` and serde fix its wire tag, shape and content address. Each
/// listed type must implement `gantz_nodetag::NodeTag`, `serde::Serialize`
/// and `serde::de::DeserializeOwned`. The calling crate must depend on
/// `serde`.
///
/// Reifying data whose tag is not listed fails with a
/// `gantz_core::data::ReifyNodeError` naming the tag.
///
/// ```ignore
/// pub fn codec() -> gantz_edit::codec::NodeCodec<Box<dyn gantz_core::Node>> {
///     gantz_edit::node_codec! {
///         Box<dyn gantz_core::Node>, |n| n as &dyn std::any::Any;
///         NodeSet {
///             gantz_core::node::Expr,
///             // ...
///         }
///     }
/// }
/// ```
#[macro_export]
macro_rules! node_codec {
    ($node:ty, |$n:ident| $as_any:expr; $carrier:ty { $($ty:ty),+ $(,)? }) => {{
        fn reify(
            node_data: &$crate::gantz_ca::NodeData,
        ) -> ::std::result::Result<
            $crate::codec::NodeInstance<$node>,
            $crate::gantz_core::data::ReifyNodeError,
        > {
            $(
                if node_data.tag == <$ty as $crate::gantz_nodetag::NodeTag>::TAG {
                    return $crate::gantz_core::data::reify_node_concrete::<$ty>(node_data)
                        .map(|node| {
                            let node: $node = ::std::boxed::Box::new(node);
                            $crate::codec::NodeInstance::new(node, |$n: &$node| {
                                let $n: &$node = $n;
                                $crate::codec::erase_as::<$ty>($as_any)
                            })
                        });
                }
            )+
            ::std::result::Result::Err($crate::gantz_core::data::ReifyNodeError {
                tag: node_data.tag.clone(),
                source: <$crate::gantz_ca::DatumError as ::serde::de::Error>::custom(
                    "unknown node type tag: not listed in the node codec",
                ),
            })
        }
        fn normalize(
            node_data: &$crate::gantz_ca::NodeData,
        ) -> ::std::result::Result<$crate::gantz_ca::NodeData, $crate::codec::NormalizeNodeError>
        {
            ::std::result::Result::Ok(reify(node_data)?.erase()?)
        }
        $crate::codec::NodeCodec::new(
            reify,
            $crate::codec::DataCodec::new(
                normalize,
                <$carrier as $crate::gantz_format::NodeSugar>::sugar,
            ),
        )
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NodeSet;

    impl gantz_format::NodeSugar for NodeSet {
        fn sugar() -> gantz_format::Sugars<'static> {
            gantz_format::Sugars(vec![&gantz_format::CoreSugar])
        }
    }

    type Node = Box<dyn gantz_core::Node>;

    fn codec() -> NodeCodec<Node> {
        crate::node_codec! {
            Node, |node| {
                let node: &dyn gantz_core::Node = &**node;
                node as &dyn Any
            };
            NodeSet {
                gantz_core::node::Expr,
                gantz_core::node::Identity,
            }
        }
    }

    /// A listed node reifies to its type and erases back to the same data. An
    /// unlisted tag fails to reify and to normalize.
    #[test]
    fn listed_nodes_round_trip_and_unlisted_tags_fail() {
        let codec = codec();
        let expr = gantz_core::node::Expr::new("(+ 1 2)").unwrap();
        let data = gantz_core::data::erase_node_typed(&expr).unwrap();
        let inst = codec.reify(&data).unwrap();
        let any: &dyn Any = &*inst.node;
        assert!(any.is::<gantz_core::node::Expr>());
        assert_eq!(inst.erase().unwrap(), data);
        assert_eq!(codec.data().normalize(&data).unwrap(), data);

        let unknown = NodeData::new("Unknown", gantz_ca::Datum::Map(vec![]));
        assert_eq!(codec.reify(&unknown).err().unwrap().tag, "Unknown");
        assert!(codec.normalize(&unknown).is_err());
    }
}
