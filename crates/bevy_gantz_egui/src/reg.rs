//! The typed side of the registry: the reified-graph cache, the builtin
//! palette and node lookup, all concrete over the erased UI node
//! ([`DynNode`]).

use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head::{HeadRef, OpenHead};
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::Node;
use gantz_core::data::ReifiedGraphs;
use gantz_egui::node::{DynNode, NodeCodec, UiBuiltins};

use crate::NodeCodecRes;

/// A `Resource` wrapper around the append-only reified-graph cache serving
/// the registry's graphs as typed `Graph<DynNode>`s.
///
/// Keep it in step with the registry via [`refresh_cache`] wherever the
/// registry is mutated before typed reads happen.
#[derive(Default, Resource)]
pub struct GraphCache(pub ReifiedGraphs<DynNode>);

/// Resource carrying the app's builtin palette: the composed
/// [`Builtins`](gantz_core::Builtins) data plus one reified node instance per
/// builtin, keyed by its erased content address.
///
/// The instances serve both compilation (via [`lookup_node`]'s
/// addr -> `&dyn Node` fallback) and the GUI's introspection (palette docs,
/// socket previews) - one map for both.
#[derive(Default, Resource)]
pub struct BuiltinNodes {
    /// The composed builtin palette as data.
    pub builtins: gantz_core::Builtins,
    /// One reified instance per builtin, keyed by erased content address.
    pub instances: UiBuiltins,
}

impl std::ops::Deref for GraphCache {
    type Target = ReifiedGraphs<DynNode>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for GraphCache {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl BuiltinNodes {
    /// Reify one instance per builtin through the codec.
    ///
    /// Failures are returned for logging; a builtin that fails to reify
    /// (e.g. a tag from a domain not compiled in) degrades to a lookup miss.
    pub fn reify(
        builtins: gantz_core::Builtins,
        codec: &NodeCodec,
    ) -> (Self, Vec<gantz_core::data::ReifyNodeError>) {
        let (instances, errs) = UiBuiltins::reify(&builtins, codec);
        (
            Self {
                builtins,
                instances,
            },
            errs,
        )
    }
}

/// Look up a node by content address.
///
/// Checks reified registry graphs first (a graph in the registry IS a node),
/// then falls back to the reified builtin instances (see [`BuiltinNodes`]).
pub fn lookup_node<'a>(
    cache: &'a ReifiedGraphs<DynNode>,
    builtins: &'a UiBuiltins,
    ca: &ca::ContentAddr,
) -> Option<&'a dyn Node> {
    let graph_ca = ca::GraphAddr::from(*ca);
    if let Some(graph) = cache.get(&graph_ca) {
        return Some(graph as &dyn Node);
    }
    builtins.get(ca).map(|n| &**n as &dyn Node)
}

/// The [`gantz_egui::Env`] over the app's resources: the shared immutable
/// input to the widgets and nodes.
pub fn env<'a>(
    registry: &'a Registry,
    cache: &'a GraphCache,
    builtins: &'a BuiltinNodes,
    codec: &'a NodeCodecRes,
) -> gantz_egui::Env<'a> {
    gantz_egui::Env {
        registry,
        builtins: &builtins.builtins,
        codec: &codec.0,
        graphs: cache,
        instances: &builtins.instances,
    }
}

/// Bring the reified-graph cache up to date with the registry, best effort.
///
/// Graphs that fail to reify (e.g. an unknown tag from a domain not compiled
/// in) are logged and remain cache misses that typed lookups degrade over.
pub fn refresh_cache(reg: &Registry, cache: &mut GraphCache, codec: &NodeCodec) {
    let reify = |nd: &ca::NodeData| codec.reify_ui(nd).map(|inst| inst.node);
    for e in cache.0.ensure_all_with(reg, reify) {
        log::error!("failed to reify registry graph: {e}");
    }
}

/// Prune unreachable content and metadata from the registry, dropping the
/// pruned graphs' cache entries with them.
pub fn prune_unused(
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    heads: Query<&HeadRef, With<OpenHead>>,
) {
    let extra = heads
        .iter()
        .filter_map(|h| registry.head_commit_ca(&**h))
        .collect::<Vec<_>>();
    let live = ca::closure(&registry.0, extra);
    ca::prune(&mut registry.0, &live);
    cache.0.retain_live(&live);
}
