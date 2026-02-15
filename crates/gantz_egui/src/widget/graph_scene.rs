use crate::{Cmd, NodeUi, Registry};
use egui_graph::{
    self,
    node::{EdgeEvent, SocketKind},
};
use gantz_core::{
    Edge, Node,
    node::{self, graph::Graph},
};
use petgraph::{
    self,
    visit::{EdgeRef, IntoNodeIdentifiers, NodeIndexable},
};
use std::collections::HashSet;
use steel::steel_vm::engine::Engine;

/// Response from the [`GraphScene`] widget.
pub struct GraphSceneResponse {
    /// The response from the underlying scene widget.
    pub scene: egui::Response,
    /// Responses from each node, keyed by node index.
    pub nodes: Vec<(NodeIndex, NodeResponse)>,
}

/// An alias for the node response type returned from gantz nodes.
pub type NodeResponse = egui_graph::node::NodeResponse<egui::Response>;

impl GraphSceneResponse {
    /// Returns true if any node was clicked.
    pub fn any_node_clicked(&self) -> bool {
        self.nodes.iter().any(|(_, r)| r.clicked())
    }

    /// Returns true if any node is being interacted with (clicked, dragged, changed, etc).
    pub fn any_node_interacted(&self) -> bool {
        self.nodes
            .iter()
            .any(|(_, r)| r.clicked() || r.dragged() || r.changed())
    }
}

/// For node types that may represent a nested [`Graph`].
pub trait ToGraphMut {
    /// The type of the node used within the [`Graph`].
    type Node;
    /// If this node is a nested graph, return a mutable reference to it.
    fn to_graph_mut(&mut self) -> Option<&mut Graph<Self::Node>>;
}

pub type EdgeIndex = petgraph::graph::EdgeIndex<usize>;
pub type NodeIndex = petgraph::graph::NodeIndex<usize>;

/// A widget used for presenting a graph scene for viewing and manipulating a
/// gantz graph.
pub struct GraphScene<'a, N> {
    registry: &'a dyn Registry,
    graph: &'a mut Graph<N>,
    path: &'a [node::Id],
    id: egui::Id,
    auto_layout: bool,
    layout_flow: egui::Direction,
    center_view: bool,
}

/// State associated with the [`GraphScene`] widget that can be useful to access
/// outside the widget.
#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct GraphSceneState {
    pub interaction: Interaction,
    /// Commands queued within the graph scene widget to be handled externally.
    #[serde(default, skip)]
    pub cmds: Vec<Cmd>,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct Interaction {
    pub selection: Selection,
    #[serde(default, skip)]
    pub edge_in_progress: Option<(NodeIndex, SocketKind, usize)>,
    /// Position where an edge context menu was opened (in graph coordinates).
    #[serde(default, skip)]
    pub edge_context_menu_pos: Option<egui::Pos2>,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct Selection {
    pub nodes: HashSet<NodeIndex>,
    pub edges: HashSet<EdgeIndex>,
}

impl<'a, N> GraphScene<'a, N>
where
    N: Node + NodeUi,
{
    /// Create a graph scene for the given graph that resides at the given path
    /// from the root.
    ///
    /// E.g. to present the root graph, provide the root graph and an empty
    /// slice.
    ///
    /// NOTE: this means the `path` is not an index into the graph, but is the
    /// path that this braph resides at within some root graph.
    pub fn new(registry: &'a dyn Registry, graph: &'a mut Graph<N>, path: &'a [node::Id]) -> Self {
        Self {
            registry,
            graph,
            path,
            id: egui::Id::new("gantz-graph-scene"),
            auto_layout: false,
            layout_flow: egui::Direction::TopDown,
            center_view: false,
        }
    }

    /// Use the given ID for the graph scene.
    ///
    /// Default: `egui::Id::new("gantz-graph-scene")`
    pub fn with_id(mut self, id: egui::Id) -> Self {
        self.id = id;
        self
    }

    /// Whether or not to automatically layout the graph using
    /// [`egui_graph::layout()`].
    ///
    /// Default: `false`
    pub fn auto_layout(mut self, auto: bool) -> Self {
        self.auto_layout = auto;
        self
    }

    /// The direction in which the egui_graph autolayout.
    ///
    /// Default: [`egui::Direction::TopDown`]
    pub fn layout_flow(mut self, flow: egui::Direction) -> Self {
        self.layout_flow = flow;
        self
    }

    /// Whether or not to center the view over the graph.
    ///
    /// Default: `false`
    pub fn center_view(mut self, center: bool) -> Self {
        self.center_view = center;
        self
    }

    /// Show the graph scene.
    ///
    /// Returns a response containing both the scene response and all node responses.
    pub fn show(
        self,
        view: &mut egui_graph::View,
        state: &mut GraphSceneState,
        vm: &mut Engine,
        ui: &mut egui::Ui,
    ) -> GraphSceneResponse {
        if self.auto_layout {
            view.layout = layout(&*self.graph, self.id, self.layout_flow, ui.ctx());
        }
        let mut node_responses = Vec::new();
        let selected: HashSet<egui_graph::NodeId> = state
            .interaction
            .selection
            .nodes
            .iter()
            .map(|ix| egui_graph::NodeId::from_u64(ix.index() as u64))
            .collect();
        let scene = egui_graph::Graph::from_id(self.id)
            .center_view(self.center_view)
            .selected_nodes(selected)
            .show(view, ui, |ui, show| {
                show.nodes(ui, |nctx, ui| {
                    node_responses =
                        nodes(self.registry, self.graph, self.path, nctx, state, vm, ui);
                })
                .edges(ui, |ectx, ui| edges(self.graph, self.path, ectx, state, ui));
            })
            .response;
        GraphSceneResponse {
            scene,
            nodes: node_responses,
        }
    }
}

impl Selection {
    pub fn clear(&mut self) {
        self.edges.clear();
        self.nodes.clear();
    }
}

/// Produce the layout for the given graph.
///
/// The `graph_id` is used to scope node IDs so that nodes with the same index
/// in different graphs don't share egui memory state.
pub fn layout<N>(
    graph: &Graph<N>,
    graph_id: egui::Id,
    flow: egui::Direction,
    ctx: &egui::Context,
) -> egui_graph::Layout {
    if graph.node_count() == 0 {
        return Default::default();
    }
    let nodes_vec = egui_graph::with_graph_memory(ctx, graph_id, |gmem| {
        let node_sizes = gmem.node_sizes();
        graph
            .node_indices()
            .map(|n| {
                let node_id = egui_graph::NodeId::from_u64(n.index() as u64);
                let size = node_sizes
                    .get(&node_id)
                    .cloned()
                    .unwrap_or_else(|| [200.0, 50.0].into());
                (node_id, size)
            })
            .collect::<Vec<_>>()
    });
    let nodes = nodes_vec.into_iter();
    let edges = graph
        .edge_indices()
        .filter_map(|e| graph.edge_endpoints(e))
        .map(|(a, b)| {
            (
                egui_graph::NodeId::from_u64(a.index() as u64),
                egui_graph::NodeId::from_u64(b.index() as u64),
            )
        });
    egui_graph::layout(nodes, edges, flow)
}

fn nodes<N>(
    registry: &dyn Registry,
    graph: &mut Graph<N>,
    path: &[node::Id],
    nctx: &mut egui_graph::NodesCtx,
    state: &mut GraphSceneState,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) -> Vec<(NodeIndex, NodeResponse)>
where
    N: Node + NodeUi,
{
    // Create meta context using registry for proper node lookup.
    let get_node = |ca: &gantz_ca::ContentAddr| registry.node(ca);
    let meta_ctx = gantz_core::node::MetaCtx::new(&get_node);
    let node_ids: Vec<_> = graph.node_identifiers().collect();
    let mut path = path.to_vec();
    let (inlets, outlets) = crate::inlet_outlet_ids(registry, graph);
    let mut responses = Vec::with_capacity(node_ids.len());
    for n_id in node_ids {
        let n_ix = graph.to_index(n_id);
        let node = &mut graph[n_id];
        let inputs = node.n_inputs(meta_ctx);
        let outputs = node.n_outputs(meta_ctx);
        let node_id = egui_graph::NodeId::from_u64(n_ix as u64);
        let response = egui_graph::node::Node::from_id(node_id)
            .inputs(inputs)
            .outputs(outputs)
            .flow(node.flow(registry))
            .show(nctx, ui, |nui_ctx| {
                path.push(n_ix);

                // Create the gantz node context.
                let node_ctx =
                    crate::NodeCtx::new(registry, &path, &inlets, &outlets, vm, &mut state.cmds);

                // Instantiate the node UI, return its response.
                let response = node.ui(node_ctx, nui_ctx);

                path.pop();
                response
            });

        // Always update the selected nodes to stay in sync with egui_graph.
        // TODO: Remove this workaround once egui_graph#47 is fixed.
        if egui_graph::is_node_selected(ui, nctx.graph_id, node_id) {
            state.interaction.selection.nodes.insert(n_id);
        } else {
            state.interaction.selection.nodes.remove(&n_id);
        }

        if response.changed() {
            // Check for an edge event.
            if let Some(ev) = response.edge_event() {
                match ev {
                    EdgeEvent::Started { kind, index } => {
                        state.interaction.edge_in_progress = Some((n_id, kind, index));
                    }
                    EdgeEvent::Ended { kind, index } => {
                        // Create the edge.
                        if let Some((src, _, ix)) = state.interaction.edge_in_progress.take() {
                            let (index, ix) = (index as u16, ix as u16);
                            let (a, b, w) = match kind {
                                SocketKind::Input => (src, n_id, Edge::from((ix, index))),
                                SocketKind::Output => (n_id, src, Edge::from((index, ix))),
                            };
                            // Check that this edge doesn't already exist.
                            if !graph.edges(a).any(|e| e.target() == b && *e.weight() == w) {
                                graph.add_edge(a, b, w);
                            }
                        }
                    }
                    EdgeEvent::Cancelled => {
                        state.interaction.edge_in_progress = None;
                    }
                }
            }

            // If the delete key was pressed while selected, remove it.
            if response.removed() {
                graph.remove_node(n_id);
            }
        }

        responses.push((n_id, response));
    }
    responses
}

fn edges<N>(
    graph: &mut Graph<N>,
    path: &[node::Id],
    ectx: &mut egui_graph::EdgesCtx,
    state: &mut GraphSceneState,
    ui: &mut egui::Ui,
) {
    // Track whether any edge has a context menu open this frame.
    let mut any_context_menu_open = false;

    // Instantiate all edges.
    for e in graph.edge_indices().collect::<Vec<_>>() {
        let (na, nb) = graph.edge_endpoints(e).unwrap();
        let edge = *graph.edge_weight(e).unwrap();
        let (input, output) = (edge.input.0.into(), edge.output.0.into());
        let a = egui_graph::NodeId::from_u64(na.index() as u64);
        let b = egui_graph::NodeId::from_u64(nb.index() as u64);
        let mut selected = state.interaction.selection.edges.contains(&e);
        let response =
            egui_graph::edge::Edge::new((a, output), (b, input), &mut selected).show(ectx, ui);

        if response.deleted() {
            graph.remove_edge(e);
            state.interaction.selection.edges.remove(&e);
        } else if response.changed() {
            if selected {
                state.interaction.selection.edges.insert(e);
            } else {
                state.interaction.selection.edges.remove(&e);
            }
        }

        // Context menu for edges - must be called every frame to keep menu open.
        // Only store position on the FIRST frame the context menu opens.
        // If we already have a position stored, don't overwrite it (the pointer
        // may have moved to the context menu popup, causing closest_point to
        // become invalid).
        let context_menu_open = response.context_menu_opened();
        if context_menu_open {
            any_context_menu_open = true;
            if state.interaction.edge_context_menu_pos.is_none() {
                state.interaction.edge_context_menu_pos = Some(response.closest_point());
            }
        }
        response.context_menu(|ui| {
            if ui.button("inspect").clicked() {
                if let Some(pos) = state.interaction.edge_context_menu_pos.take() {
                    state.cmds.push(Cmd::InspectEdge(crate::InspectEdge {
                        path: path.to_vec(),
                        edge: e,
                        pos,
                    }));
                }
                ui.close();
            }
        });
    }

    // Clear the stored position if no context menu is open (user dismissed it).
    if !any_context_menu_open {
        state.interaction.edge_context_menu_pos = None;
    }

    // Draw the in-progress edge if there is one.
    if let Some(edge) = ectx.in_progress(ui) {
        edge.show(ui);
    }
}

/// Index into the given graph using the given path.
///
/// Returns `None` in the case that `path` is empty, or if there is no node at a
/// given `node::Id` in the path.
pub fn index_path_node_mut<'a, N>(graph: &'a mut Graph<N>, path: &[node::Id]) -> Option<&'a mut N>
where
    N: ToGraphMut<Node = N>,
{
    if path.is_empty() {
        return None;
    }

    let node_id = petgraph::graph::NodeIndex::new(path[0]);
    let node = graph.node_weight_mut(node_id)?;
    if path.len() == 1 {
        // If this is the end of the path, return the node
        return Some(node);
    }

    // If there are more elements in the path, this node should be a graph node
    // Try to get the nested graph and continue traversing
    let nested = node.to_graph_mut()?;
    index_path_node_mut(nested, &path[1..])
}

/// Index into the given graph using the given path.
///
/// Returns `None` in the case that `path` is empty, or if there is no node at a
/// given `node::Id` in the path.
pub fn index_path_graph_mut<'a, N>(
    graph: &'a mut Graph<N>,
    path: &[node::Id],
) -> Option<&'a mut Graph<N>>
where
    N: ToGraphMut<Node = N>,
{
    if path.is_empty() {
        return Some(graph);
    }
    index_path_node_mut(graph, path).and_then(|node| node.to_graph_mut())
}
