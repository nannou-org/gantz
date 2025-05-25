use crate::{Cmd, NodeUi};
use egui_graph::{
    self,
    node::{EdgeEvent, SocketKind},
};
use gantz_core::{Edge, Node};
use petgraph::{
    self,
    visit::{EdgeRef, IntoNodeIdentifiers, NodeIndexable},
};
use std::collections::{HashMap, HashSet};
use steel::steel_vm::engine::Engine;

/// The graph type excepted by the `GraphScene` widget.
// TODO: Provide a `Graph` trait instead for more flexibility?
pub type Graph<N> = petgraph::stable_graph::StableGraph<N, Edge, petgraph::Directed, usize>;

pub type EdgeIndex = petgraph::graph::EdgeIndex<usize>;
pub type NodeIndex = petgraph::graph::NodeIndex<usize>;

/// A widget used for presenting a graph scene for viewing and manipulating a
/// gantz graph.
pub struct GraphScene<'a, N> {
    graph: &'a mut Graph<N>,
    id: egui::Id,
    auto_layout: bool,
    layout_flow: egui::Direction,
    center_view: bool,
}

/// State associated with the [`GraphScene`] widget that can be useful to access
/// outside the widget.
#[derive(Default)]
pub struct GraphSceneState {
    pub node_id_map: HashMap<egui::Id, NodeIndex>,
    pub interaction: Interaction,
    /// Commands queued within the graph scene widget to be handled externally.
    pub cmds: Vec<Cmd>,
}

#[derive(Default)]
pub struct Interaction {
    pub selection: Selection,
    pub edge_in_progress: Option<(NodeIndex, SocketKind, usize)>,
}

#[derive(Default)]
pub struct Selection {
    pub nodes: HashSet<NodeIndex>,
    pub edges: HashSet<EdgeIndex>,
}

impl<'a, N> GraphScene<'a, N>
where
    N: Node + NodeUi,
{
    /// Create a graph scene for the given gantz graph.
    pub fn new(graph: &'a mut Graph<N>) -> Self {
        Self {
            graph,
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
    pub fn show(
        self,
        view: &mut egui_graph::View,
        state: &mut GraphSceneState,
        vm: &mut Engine,
        ui: &mut egui::Ui,
    ) {
        if self.auto_layout {
            view.layout = layout(&*self.graph, self.layout_flow, ui.ctx());
        }
        egui_graph::Graph::new(self.id)
            .center_view(self.center_view)
            .show(view, ui, |ui, show| {
                show.nodes(ui, |nctx, ui| nodes(self.graph, nctx, state, vm, ui))
                    .edges(ui, |ectx, ui| edges(self.graph, ectx, state, ui));
            });
    }
}

/// Produce the layout for the given graph.
pub fn layout<N>(
    graph: &Graph<N>,
    flow: egui::Direction,
    ctx: &egui::Context,
) -> egui_graph::Layout {
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

fn nodes<N>(
    graph: &mut Graph<N>,
    nctx: &mut egui_graph::NodesCtx,
    state: &mut GraphSceneState,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) where
    N: Node + NodeUi,
{
    let node_ids: Vec<_> = graph.node_identifiers().collect();
    for n_id in node_ids {
        let n_ix = graph.to_index(n_id);
        let node = &mut graph[n_id];
        let inputs = node.n_inputs();
        let outputs = node.n_outputs();
        let egui_id = egui::Id::new(n_id);
        state.node_id_map.insert(egui_id, n_id);
        let response = egui_graph::node::Node::from_id(egui_id)
            .inputs(inputs)
            .outputs(outputs)
            .flow(node.flow())
            .show(nctx, ui, |ui| {
                let path = vec![n_ix];
                let node_ctx = crate::NodeCtx::new(&path, vm, &mut state.cmds);

                // Instantiate the node's UI.
                node.ui(node_ctx, ui);
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
                state.node_id_map.remove(&egui_id);
            }
        }
    }
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
