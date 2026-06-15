use crate::{
    CopyNodes, InspectEdge, NodeUi, OpenCommandPalette, OpenHead, Paste, PastePos, Registry,
    response::DynResponse,
};
use egui::emath::GuiRounding;
use egui_graph::{self, SocketKind, node::EdgeEvent};
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
    /// Dynamic payloads emitted within the scene (node UIs, context menus),
    /// to be handled by the application after the pass.
    pub responses: Vec<DynResponse>,
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

pub type EdgeIndex = petgraph::graph::EdgeIndex<usize>;
pub type NodeIndex = petgraph::graph::NodeIndex<usize>;

/// A widget used for presenting a graph scene for viewing and manipulating a
/// gantz graph.
pub struct GraphScene<'a, N> {
    registry: &'a dyn Registry,
    graph: &'a mut Graph<N>,
    id: egui::Id,
    auto_layout: bool,
    layout_flow: egui::Direction,
    center_view: bool,
    immutable: bool,
}

/// State associated with the [`GraphScene`] widget that can be useful to access
/// outside the widget.
#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct GraphSceneState {
    pub interaction: Interaction,
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
    /// Create a graph scene for the given graph (a head's root graph; nested
    /// graphs are separate heads).
    pub fn new(registry: &'a dyn Registry, graph: &'a mut Graph<N>) -> Self {
        Self {
            registry,
            graph,
            id: egui::Id::new("gantz-graph-scene"),
            auto_layout: false,
            layout_flow: egui::Direction::TopDown,
            center_view: false,
            immutable: false,
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

    /// Set immutable (view-only) mode.
    ///
    /// When `true`, prevents structural changes (node dragging, edge
    /// creation/deletion, node deletion, content editing) while preserving
    /// navigation and selection.
    ///
    /// Default: `false`
    pub fn immutable(mut self, immutable: bool) -> Self {
        self.immutable = immutable;
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
        let mut responses: Vec<DynResponse> = Vec::new();
        let selected: HashSet<egui_graph::NodeId> = state
            .interaction
            .selection
            .nodes
            .iter()
            .map(|ix| egui_graph::NodeId::from_u64(ix.index() as u64))
            .collect();
        let graph_response = egui_graph::Graph::from_id(self.id)
            .center_view(self.center_view)
            .selected_nodes(selected)
            .immutable(self.immutable)
            .show(view, ui, |ui, show| {
                let immutable = self.immutable;
                show.nodes(ui, |nctx, ui| {
                    node_responses = nodes(
                        self.registry,
                        self.graph,
                        nctx,
                        state,
                        &mut responses,
                        vm,
                        immutable,
                        ui,
                    );
                })
                .edges(ui, |ectx, ui| {
                    edges(self.graph, ectx, state, &mut responses, ui)
                });
            });

        // Sync selection when egui_graph reports a change.
        if let Some(selected) = graph_response.selection_changed {
            state.interaction.selection.nodes = selected
                .into_iter()
                .map(|id| NodeIndex::new(id.value() as usize))
                .collect();
        }

        // Background context menu.
        if !self.immutable {
            let layer_id = graph_response.response.layer_id;
            graph_response.response.context_menu(|ui| {
                // The popup is placed at the right-click location, so its
                // top-left corner in screen space corresponds to where the
                // user clicked. Convert that to graph space for paste
                // positioning.
                let menu_screen_pos = ui.min_rect().left_top();
                if ui.button("add node").clicked() {
                    responses.push(DynResponse::new(OpenCommandPalette));
                    ui.close();
                }
                if ui.button("paste").clicked() {
                    let graph_pos = ui
                        .ctx()
                        .layer_transform_from_global(layer_id)
                        .map(|t| t * menu_screen_pos)
                        .unwrap_or(menu_screen_pos);
                    let pos = PastePos::GraphPos(graph_pos);
                    responses.push(DynResponse::new(Paste { text: None, pos }));
                    ui.close();
                }
            });
        }

        GraphSceneResponse {
            scene: graph_response.response,
            nodes: node_responses,
            responses,
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
    nctx: &mut egui_graph::NodesCtx,
    state: &mut GraphSceneState,
    responses: &mut Vec<DynResponse>,
    vm: &mut Engine,
    immutable: bool,
    ui: &mut egui::Ui,
) -> Vec<(NodeIndex, NodeResponse)>
where
    N: Node + NodeUi,
{
    // Create meta context using registry for proper node lookup.
    let get_node = |ca: &gantz_ca::ContentAddr| registry.node(ca);
    let meta_ctx = gantz_core::node::MetaCtx::new(&get_node);
    let node_ids: Vec<_> = graph.node_identifiers().collect();
    let (inlets, outlets) = crate::inlet_outlet_ids(registry, graph);
    let mut node_responses = Vec::with_capacity(node_ids.len());
    let mut nodes_to_delete = Vec::new();
    let mut nodes_to_reset = Vec::new();
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
            .max_width(f32::INFINITY)
            .show(nctx, ui, |nui_ctx| {
                // A node at this (root) level has the single-element state path
                // `[n_ix]`.
                let node_path = [n_ix];
                let node_ctx =
                    crate::NodeCtx::new(registry, &node_path, &inlets, &outlets, vm, responses);
                node.ui(node_ctx, nui_ctx)
            });

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

            // If the delete key was pressed while selected, defer removal.
            if response.removed() {
                nodes_to_delete.push(n_id);
            }
        }

        // Node context menu.
        response.context_menu(|ui| {
            let selected = &state.interaction.selection.nodes;
            let target: HashSet<NodeIndex> = if selected.contains(&n_id) {
                selected.clone()
            } else {
                HashSet::from([n_id])
            };
            if ui.button("copy").clicked() {
                responses.push(DynResponse::new(CopyNodes(target.clone())));
                ui.close();
            }
            // Demo graph, if the node has one.
            let demo_name = graph[n_id].demo_graph(registry);
            let demo_btn = ui.add_enabled(demo_name.is_some(), egui::Button::new("demo"));
            if let Some(name) = demo_name {
                if demo_btn.on_hover_text(format!("opens {name}")).clicked() {
                    responses.push(DynResponse::new(OpenHead(gantz_ca::Head::Branch(
                        name.to_string(),
                    ))));
                    ui.close();
                }
            } else {
                demo_btn.on_disabled_hover_text("no associated demo");
            }
            // "open in new tab" for nodes that reference a named graph.
            if let Some(head) = graph[n_id].nav_head(registry) {
                if ui
                    .button("open tab")
                    .on_hover_text("open the referenced graph in a new tab")
                    .clicked()
                {
                    responses.push(DynResponse::new(OpenHead(head)));
                    ui.close();
                }
            }
            if !immutable {
                let stateful = target
                    .iter()
                    .any(|&n| graph.contains_node(n) && graph[n].stateful(meta_ctx));
                let reset_btn = ui.add_enabled(stateful, egui::Button::new("reset"));
                if reset_btn
                    .on_hover_text("reset the node to its default state")
                    .on_disabled_hover_text("no state to reset")
                    .clicked()
                {
                    nodes_to_reset.extend(target.iter().copied());
                    ui.close();
                }
                if ui.button("delete").clicked() {
                    nodes_to_delete.extend(target);
                    ui.close();
                }
            }
        });

        node_responses.push((n_id, response));
    }

    // Unified delete: both keyboard and context menu deletes go through here.
    for n_id in nodes_to_delete {
        if graph.contains_node(n_id) {
            let _ = gantz_core::node::state::remove_value(vm, &[n_id.index()]);
            graph.remove_node(n_id);
            state.interaction.selection.nodes.remove(&n_id);
        }
    }

    // Reset state by removing it, then re-registering the graph.
    // Registration is idempotent and re-initialises any missing state.
    if !nodes_to_reset.is_empty() {
        for n_id in nodes_to_reset {
            if graph.contains_node(n_id) {
                let _ = gantz_core::node::state::remove_value(vm, &[n_id.index()]);
            }
        }
        gantz_core::graph::register(&get_node, &*graph, &[], vm);
    }

    node_responses
}

fn edges<N>(
    graph: &mut Graph<N>,
    ectx: &mut egui_graph::EdgesCtx,
    state: &mut GraphSceneState,
    responses: &mut Vec<DynResponse>,
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
                    responses.push(DynResponse::new(InspectEdge { edge: e, pos }));
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

/// The id of the node to flag at the viewed level for a diagnostic path: the
/// next id under the level, so diagnostics within nested graphs flag the
/// enclosing graph node. `None` when the path is empty or lies outside the
/// level.
fn diagnostic_node_at_level(diag_path: &[node::Id], level: &[node::Id]) -> Option<node::Id> {
    diag_path.strip_prefix(level)?.first().copied()
}

/// Paint a glow and hover message over the nodes implicated by diagnostics,
/// and tint the scene border when any diagnostic cannot be attributed to a
/// node visible at this level (e.g. a whole-graph error, or one in a level
/// not currently viewed).
pub fn paint_diagnostics(
    diagnostics: &[gantz_core::Diagnostic],
    level: &[node::Id],
    response: &GraphSceneResponse,
    ui: &egui::Ui,
) {
    if diagnostics.is_empty() {
        return;
    }
    let color = ui.visuals().error_fg_color;
    let mut unattributed = false;
    for diag in diagnostics {
        let flagged = diagnostic_node_at_level(&diag.path, level).and_then(|flag| {
            let ix = response
                .nodes
                .iter()
                .position(|(ix, _)| ix.index() == flag)?;
            Some(&response.nodes[ix].1)
        });
        let Some(node_response) = flagged else {
            unattributed = true;
            continue;
        };
        // Paint on the node's own layer so the scene transform applies.
        // Everything the painter receives must be in layer-local
        // coordinates: egui maps a transformed layer's clip rects through
        // its transform at tessellation, so the pane's (global) clip is
        // mapped into local space rather than intersected as-is.
        let to_global = ui
            .ctx()
            .layer_transform_to_global(node_response.layer_id)
            .unwrap_or(egui::emath::TSTransform::IDENTITY);
        let inv = to_global.inverse();
        // The tessellator snaps each rect to physical pixels independently
        // (`round_rects_to_pixels`), which at fractional transforms lets the
        // rings land +-1px off the frame's own snapped edges. Snap the frame
        // rect the same way the frame's shape will be, derive the rings from
        // it, and disable their re-rounding so the gap stays even.
        let ppp = ui.ctx().pixels_per_point();
        let frame = inv.mul_rect(to_global.mul_rect(node_response.rect).round_to_pixels(ppp));
        let mut painter = ui.ctx().layer_painter(node_response.layer_id);
        let local_clip = inv.mul_rect(ui.clip_rect());
        painter.set_clip_rect(local_clip.intersect(frame.expand(16.0)));
        // A soft glow: thin rings hugging the frame, fading quickly. Ring
        // corners grow from the node frame's radius (`Frame::window`, see
        // `egui_graph::node::default_frame`) so the arcs stay concentric.
        let frame_radius = ui.visuals().window_corner_radius;
        let rings = [(1.0f32, 1.5, 0.45), (3.0, 2.0, 0.16), (5.5, 2.5, 0.06)];
        for (expand, width, alpha) in rings {
            painter.add(
                egui::epaint::RectShape::stroke(
                    frame.expand(expand),
                    frame_radius + expand.round() as u8,
                    egui::Stroke::new(width, color.gamma_multiply(alpha)),
                    egui::StrokeKind::Outside,
                )
                .with_round_to_pixels(false),
            );
        }
        (**node_response).clone().on_hover_text(&diag.message);
    }
    if unattributed {
        ui.painter().rect_stroke(
            response.scene.rect,
            0,
            egui::Stroke::new(2.0, color.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::diagnostic_node_at_level;

    #[test]
    fn diagnostic_level_resolution() {
        // A node at the viewed level flags itself.
        assert_eq!(diagnostic_node_at_level(&[3], &[]), Some(3));
        // A node within a nested graph flags the enclosing graph node.
        assert_eq!(diagnostic_node_at_level(&[3, 2, 1], &[]), Some(3));
        // Viewing inside the nested graph flags the inner node.
        assert_eq!(diagnostic_node_at_level(&[3, 2, 1], &[3]), Some(2));
        assert_eq!(diagnostic_node_at_level(&[3, 2, 1], &[3, 2]), Some(1));
        // Outside the viewed level or empty: unattributable.
        assert_eq!(diagnostic_node_at_level(&[3, 2], &[4]), None);
        assert_eq!(diagnostic_node_at_level(&[], &[]), None);
        assert_eq!(diagnostic_node_at_level(&[3], &[3]), None);
    }
}
