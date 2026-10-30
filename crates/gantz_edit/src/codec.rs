//! The value-level codec between a node set's typed nodes and the registry's
//! [`NodeData`] representation.
//!
//! The [`NodeCodec`] carries the node set as values rather than as a
//! `Box<dyn Trait>` serde impl. It holds one [`Kind`] per node type, keyed by
//! the wire tag, and the node set's `.gantz` keyword sugar. An application
//! composes one at run time with [`NodeCodec::new`]. Its kinds are the app's
//! node-set manifest. A node type is storable exactly when its kind is in the
//! codec.
//!
//! A codec is generic over the reified node type `N`, such as a boxed trait
//! object. The text format and the export paths need no typed node at all.
//! They take the codec's N-free [`DataCodec`].

use gantz_ca::{ContentAddr, DataGraph, NodeData};
use gantz_core::Builtins;
use gantz_core::data::{EraseNodeError, ReifyError, ReifyNodeError};
use gantz_core::node::graph::Graph;
use gantz_format::Sugars;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// One node type of a node set: its wire tag, and how its stored form
/// reifies to `N`.
///
/// [`Kind::typed`] gives the kind of a Rust node type. [`Kind::new`] takes any
/// reify fn, such as one for nodes whose kind is data.
pub struct Kind<N> {
    tag: String,
    reify: Arc<ReifyFn<N>>,
}

/// A reified node paired with the eraser for its kind.
///
/// Produced by [`NodeCodec::reify`]. The kind that decoded the node knows how
/// to erase it again, so no trait-object serde is involved.
pub struct NodeInstance<N> {
    /// The reified node.
    pub node: N,
    erase: Arc<EraseFn<N>>,
}

/// The node-type-free part of a [`NodeCodec`]: normalize a node's data form
/// through its kind, and the node set's `.gantz` keyword sugar.
#[derive(Clone)]
pub struct DataCodec {
    normalize: Arc<NormalizeFn>,
    sugars: Sugars,
}

/// A value-level codec between a node set's typed nodes `N` and their stored
/// [`NodeData`] form. Build one with [`NodeCodec::new`].
pub struct NodeCodec<N> {
    kinds: Arc<HashMap<String, Kind<N>>>,
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

/// Reifies the stored form of one kind's node.
type ReifyFn<N> = dyn Fn(&NodeData) -> Result<NodeInstance<N>, ReifyNodeError> + Send + Sync;

/// Erases a node back to its stored form.
type EraseFn<N> = dyn Fn(&N) -> Result<NodeData, EraseNodeError> + Send + Sync;

/// Round-trips a stored form through its kind.
type NormalizeFn = dyn Fn(&NodeData) -> Result<NodeData, NormalizeNodeError> + Send + Sync;

impl<N: 'static> Kind<N> {
    /// The kind with wire tag `tag`, whose stored nodes reify through
    /// `reify`. The reified instance carries its own eraser.
    pub fn new(
        tag: impl Into<String>,
        reify: impl Fn(&NodeData) -> Result<NodeInstance<N>, ReifyNodeError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            tag: tag.into(),
            reify: Arc::new(reify),
        }
    }

    /// The kind of the Rust node type `T`. A node reifies through `T`'s serde
    /// and `wrap`, and erases as a `T`, which it reaches through `as_any`.
    pub fn typed<T>(wrap: fn(T) -> N, as_any: fn(&N) -> &dyn Any) -> Self
    where
        T: gantz_nodetag::NodeTag
            + serde::Serialize
            + serde::de::DeserializeOwned
            + gantz_core::Node
            + 'static,
    {
        let erase: Arc<EraseFn<N>> = Arc::new(move |node: &N| erase_as::<T>(as_any(node)));
        Self::new(T::TAG, move |node_data| {
            let node = gantz_core::data::reify_node_concrete::<T>(node_data)?;
            Ok(NodeInstance {
                node: wrap(node),
                erase: erase.clone(),
            })
        })
    }

    /// The kind's wire tag.
    pub fn tag(&self) -> &str {
        &self.tag
    }
}

impl<N> NodeInstance<N> {
    /// Pair a reified node with the eraser of its kind.
    ///
    /// `erase` must erase the node as its own kind. The kind that reifies the
    /// node keeps the two in agreement.
    pub fn new(
        node: N,
        erase: impl Fn(&N) -> Result<NodeData, EraseNodeError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            node,
            erase: Arc::new(erase),
        }
    }

    /// Erase the node back to its canonical data form. See
    /// [`gantz_core::data::erase_node_typed`].
    pub fn erase(&self) -> Result<NodeData, EraseNodeError> {
        (self.erase)(&self.node)
    }
}

impl DataCodec {
    /// Round-trip a node's data form through its kind. Reify, then erase.
    ///
    /// This validates the fields against the node's own kind. It recomputes
    /// the canonical form and the refs and blobs columns from the node's
    /// reporting.
    pub fn normalize(&self, node_data: &NodeData) -> Result<NodeData, NormalizeNodeError> {
        (self.normalize)(node_data)
    }

    /// The node set's composed `.gantz` keyword sugar.
    pub fn sugars(&self) -> &Sugars {
        &self.sugars
    }
}

impl<N: 'static> NodeCodec<N> {
    /// The codec of the node set with the given kinds and `.gantz` keyword
    /// sugar. A later kind with the tag of an earlier one replaces it.
    pub fn new(sugars: Sugars, kinds: impl IntoIterator<Item = Kind<N>>) -> Self {
        let kinds: Arc<HashMap<_, _>> = Arc::new(
            kinds
                .into_iter()
                .map(|kind| (kind.tag.clone(), kind))
                .collect(),
        );
        let normalize = {
            let kinds = kinds.clone();
            move |node_data: &NodeData| Ok(reify(&kinds, node_data)?.erase()?)
        };
        let data = DataCodec {
            normalize: Arc::new(normalize),
            sugars,
        };
        Self { kinds, data }
    }
}

impl<N> NodeCodec<N> {
    /// Reify one stored node to a typed [`NodeInstance`].
    pub fn reify(&self, node_data: &NodeData) -> Result<NodeInstance<N>, ReifyNodeError> {
        reify(&self.kinds, node_data)
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
    pub fn sugars(&self) -> &Sugars {
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

impl<N> Clone for Kind<N> {
    fn clone(&self) -> Self {
        Self {
            tag: self.tag.clone(),
            reify: self.reify.clone(),
        }
    }
}

impl<N> Clone for NodeCodec<N> {
    fn clone(&self) -> Self {
        Self {
            kinds: self.kinds.clone(),
            data: self.data.clone(),
        }
    }
}

impl<N> Default for ReifiedBuiltins<N> {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

/// Erase `node` as the concrete type `T`. The eraser of [`Kind::typed`].
///
/// Panics if `node` is not a `T`. The kind that reified the node as a `T`
/// upholds this.
fn erase_as<T>(node: &dyn Any) -> Result<NodeData, EraseNodeError>
where
    T: gantz_nodetag::NodeTag + serde::Serialize + gantz_core::Node + 'static,
{
    let node = node.downcast_ref::<T>().expect("tag-matched type");
    gantz_core::data::erase_node_typed(node)
}

/// Reify `node_data` through the kind of its tag in `kinds`.
fn reify<N>(
    kinds: &HashMap<String, Kind<N>>,
    node_data: &NodeData,
) -> Result<NodeInstance<N>, ReifyNodeError> {
    match kinds.get(&node_data.tag) {
        Some(kind) => (kind.reify)(node_data),
        None => Err(ReifyNodeError {
            tag: node_data.tag.clone(),
            source: <gantz_ca::DatumError as serde::de::Error>::custom(
                "unknown node type tag: not a kind of the node codec",
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Node = Box<dyn gantz_core::Node>;

    fn kind<T>() -> Kind<Node>
    where
        T: gantz_nodetag::NodeTag
            + serde::Serialize
            + serde::de::DeserializeOwned
            + gantz_core::Node
            + 'static,
    {
        Kind::typed::<T>(|node| Box::new(node), |node| &**node as &dyn Any)
    }

    fn codec() -> NodeCodec<Node> {
        let sugars = Sugars::default().with(gantz_format::CoreSugar);
        NodeCodec::new(
            sugars,
            [
                kind::<gantz_core::node::Expr>(),
                kind::<gantz_core::node::Identity>(),
            ],
        )
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
