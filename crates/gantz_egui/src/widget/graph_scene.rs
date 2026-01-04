use crate::{Cmd, NodeUi};
use egui_graph::{
    self,
    node::{EdgeEvent, NodeResponse, SocketKind},
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

impl GraphSceneResponse {
    /// Returns true if any node was clicked.
    pub fn any_node_clicked(&self) -> bool {
        self.nodes.iter().any(|(_, r)| r.clicked())
    }

    /// Returns true if any node is being interacted with (clicked, dragged, changed, etc).
    pub fn any_node_interacted(&self) -> bool {
        self.nodes.iter().any(|(_, r)| r.clicked() || r.dragged() || r.changed())
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
pub struct GraphScene<'a, Env, N> {
    env: &'a Env,
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
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct Selection {
    pub nodes: HashSet<NodeIndex>,
    pub edges: HashSet<EdgeIndex>,
}

impl<'a, Env, N> GraphScene<'a, Env, N>
where
    N: Node<Env> + NodeUi<Env>,
{
    /// Create a graph scene for the given graph that resides at the given path
    /// from the root.
    ///
    /// E.g. to present the root graph, provide the root graph and an empty
    /// slice.
    ///
    /// NOTE: this means the `path` is not an index into the graph, but is the
    /// path that this braph resides at within some root graph.
    pub fn new(env: &'a Env, graph: &'a mut Graph<N>, path: &'a [node::Id]) -> Self {
        Self {
            env,
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
    /// [`egui_graph::layout`].
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
            view.layout = layout(&*self.graph, self.layout_flow, ui.ctx());
        }
        let mut node_responses = Vec::new();
        let scene = egui_graph::Graph::new(self.id)
            .center_view(self.center_view)
            .show(view, ui, |ui, show| {
                show.nodes(ui, |nctx, ui| {
                    node_responses = nodes(self.env, self.graph, self.path, nctx, state, vm, ui);
                })
                .edges(ui, |ectx, ui| edges(self.graph, ectx, state, ui));
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
pub fn layout<N>(
    graph: &Graph<N>,
    flow: egui::Direction,
    ctx: &egui::Context,
) -> egui_graph::Layout {
    if graph.node_count() == 0 {
        return Default::default();
    }
    ctx.memory(|m| {
        let nodes = graph.node_indices().map(|n| {
            // FIXME: This ID doesn't directly corellate with node size.
            let id = egui::Id::new(n);
            let size = m
                .area_rect(id)
                .map(|a| a.size())
                .unwrap_or([200.0, 50.0].into());
            (id, size)
        });
        let edges = graph
            .edge_indices()
            .filter_map(|e| graph.edge_endpoints(e))
            .map(|(a, b)| (egui::Id::new(a), egui::Id::new(b)));
        egui_graph::layout(nodes, edges, flow)
    })
}

fn nodes<Env, N>(
    env: &Env,
    graph: &mut Graph<N>,
    path: &[node::Id],
    nctx: &mut egui_graph::NodesCtx,
    state: &mut GraphSceneState,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) -> Vec<(NodeIndex, NodeResponse)>
where
    N: Node<Env> + NodeUi<Env>,
{
    let node_ids: Vec<_> = graph.node_identifiers().collect();
    let mut path = path.to_vec();
    let (inlets, outlets) = crate::inlet_outlet_ids(graph);
    let mut responses = Vec::with_capacity(node_ids.len());
    for n_id in node_ids {
        let n_ix = graph.to_index(n_id);
        let node = &mut graph[n_id];
        let inputs = node.n_inputs(env);
        let outputs = node.n_outputs(env);
        let egui_id = egui::Id::new(n_id);
        let response = egui_graph::node::Node::from_id(egui_id)
            .inputs(inputs)
            .outputs(outputs)
            .flow(node.flow(env))
            .show(nctx, ui, |ui| {
                path.push(n_ix);
                // Instantiate the node's UI.
                let node_ctx =
                    crate::NodeCtx::new(env, &path, &inlets, &outlets, vm, &mut state.cmds);
                node.ui(node_ctx, ui);
                path.pop();
            });

        if response.changed() {
            // Update the selected nodes.
            if egui_graph::is_node_selected(ui, nctx.graph_id, egui_id) {
                state.interaction.selection.nodes.insert(n_id);
            } else {
                state.interaction.selection.nodes.remove(&n_id);
            }

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
    ectx: &mut egui_graph::EdgesCtx,
    state: &mut GraphSceneState,
    ui: &mut egui::Ui,
) {
    // Instantiate all edges.
    for e in graph.edge_indices().collect::<Vec<_>>() {
        let (na, nb) = graph.edge_endpoints(e).unwrap();
        let edge = *graph.edge_weight(e).unwrap();
        let (input, output) = (edge.input.0.into(), edge.output.0.into());
        let a = egui::Id::new(na);
        let b = egui::Id::new(nb);
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
