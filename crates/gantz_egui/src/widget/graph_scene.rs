use crate::{
    CopyNodes, InspectEdge, NodeUi, OpenCommandPalette, OpenHead, OpenNodeView, Paste, PastePos,
    Registry, ResetTilesLayout, SocketDoc, response::DynResponse,
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
use std::collections::{HashMap, HashSet};
use steel::steel_vm::engine::Engine;

/// Response from the [`GraphScene`] widget.
pub struct GraphSceneResponse {
    /// The response from the underlying scene widget.
    pub scene: egui::Response,
    /// Responses from each node, keyed by node index.
    pub nodes: Vec<(NodeIndex, NodeResponse)>,
    /// Whether anything CA-affecting changed about the graph this frame: a node
    /// reported a change (see [`NodeUi`]), or the scene itself edited the graph
    /// structure (added/removed an edge, deleted a node). Lets the application
    /// detect edits without re-hashing the whole graph each frame.
    pub changed: bool,
    /// Dynamic payloads emitted within the scene (node UIs, context menus),
    /// to be handled by the application after the pass.
    pub responses: Vec<DynResponse>,
    /// The index changes from any node deletions this frame, so callers can
    /// migrate their own index-keyed data (e.g. detached node views).
    pub reindex: crate::ops::Reindex,
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
    layout_params: egui_graph::LayoutParams,
    /// Global dot-grid, snapping and snap-align options, applied to the
    /// underlying `egui_graph::Graph` builder.
    scene_config: crate::widget::gantz::SceneConfig,
    immutable: bool,
    /// When set, the background context menu gains a "Panes" submenu of
    /// pane-visibility checkboxes.
    view_toggles: Option<&'a mut crate::widget::gantz::ViewToggles>,
}

/// State associated with the [`GraphScene`] widget that can be useful to access
/// outside the widget.
#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct GraphSceneState {
    pub interaction: Interaction,
    /// One-shot request to auto-layout, set by a button or context menu and
    /// consumed by [`GraphScene::show`] on the next pass. Applies to the
    /// current selection, or the whole graph when nothing is selected.
    #[serde(default, skip)]
    pub pending_auto_layout: bool,
    /// One-shot request to center the view, consumed by [`GraphScene::show`].
    #[serde(default, skip)]
    pub pending_center_view: bool,
    /// One-shot request to align the current selection, set by the node context
    /// menu and consumed by [`GraphScene::show`] on the next pass. The feature
    /// (`AlignBy`) chooses min-edge / centre / max-edge; orientation is inferred.
    #[serde(default, skip)]
    pub pending_align: Option<egui_graph::AlignBy>,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct Interaction {
    pub selection: Selection,
    #[serde(default, skip)]
    pub edge_in_progress: Option<(NodeIndex, SocketKind, usize)>,
    /// Position where an edge context menu was opened (in graph coordinates).
    #[serde(default, skip)]
    pub edge_context_menu_pos: Option<egui::Pos2>,
    /// Latest pointer position over the scene, in graph coordinates. Updated
    /// each frame the scene is hovered; used to place palette-created nodes
    /// under the pointer.
    #[serde(default, skip)]
    pub last_pointer_pos: Option<egui::Pos2>,
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
            layout_params: egui_graph::LayoutParams::new(egui::Direction::TopDown),
            scene_config: Default::default(),
            immutable: false,
            view_toggles: None,
        }
    }

    /// Use the given ID for the graph scene.
    ///
    /// Default: `egui::Id::new("gantz-graph-scene")`
    pub fn with_id(mut self, id: egui::Id) -> Self {
        self.id = id;
        self
    }

    /// The parameters used when auto-layout is invoked (one-shot, via the
    /// pending request flags on [`GraphSceneState`]).
    ///
    /// Default: [`egui_graph::LayoutParams::new`] with [`egui::Direction::TopDown`].
    pub fn layout_params(mut self, params: egui_graph::LayoutParams) -> Self {
        self.layout_params = params;
        self
    }

    /// The global dot-grid, snapping and snap-align options to apply.
    ///
    /// Default: [`crate::widget::gantz::SceneConfig::default`].
    pub fn scene_config(mut self, scene_config: crate::widget::gantz::SceneConfig) -> Self {
        self.scene_config = scene_config;
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

    /// Provide the pane-visibility toggles, adding a "Panes" submenu to the
    /// background (graph-area) context menu. Available even in immutable mode.
    pub fn view_toggles(mut self, view_toggles: &'a mut crate::widget::gantz::ViewToggles) -> Self {
        self.view_toggles = Some(view_toggles);
        self
    }

    /// Show the graph scene.
    ///
    /// Returns a response containing both the scene response and all node responses.
    pub fn show(
        self,
        scene_view: &mut crate::SceneView,
        state: &mut GraphSceneState,
        vm: &mut Engine,
        ui: &mut egui::Ui,
    ) -> GraphSceneResponse {
        // Materialise the viewport-dependent `egui_graph::View` from the
        // viewport-independent camera. The viewport must match the size
        // `egui_graph` reads (`available_rect_before_wrap`) so the fit
        // reproduces the camera's zoom exactly. The camera is recovered after
        // the pass (see `restore_egui` below).
        let viewport = ui.available_rect_before_wrap().size();
        let mut egui_view = scene_view.take_egui(viewport);
        let view = &mut egui_view;
        // Consume one-shot layout/center requests (set by buttons and context
        // menus). A request raised during this pass (e.g. by a context menu)
        // is applied on the next pass.
        let do_layout = std::mem::take(&mut state.pending_auto_layout);
        let do_center = std::mem::take(&mut state.pending_center_view);
        if do_layout {
            // Lay out the selection, or the whole graph when nothing is
            // selected. The whole-graph case centers at the origin; a subset is
            // aligned to its prior bounding-box centre.
            let target: HashSet<NodeIndex> = if state.interaction.selection.nodes.is_empty() {
                self.graph.node_indices().collect()
            } else {
                state.interaction.selection.nodes.clone()
            };
            let whole = state.interaction.selection.nodes.is_empty();
            apply_auto_layout(
                self.registry,
                &*self.graph,
                self.id,
                &self.layout_params,
                ui.ctx(),
                view,
                &target,
                whole,
            );
        }
        // One-shot align of the current selection (set by the node context
        // menu). Only the selected nodes move; orientation is inferred from
        // their spread. Selection-only: needs at least two nodes.
        if let Some(by) = std::mem::take(&mut state.pending_align) {
            let target = &state.interaction.selection.nodes;
            if target.len() > 1 {
                apply_align(self.id, ui.ctx(), view, target, by);
            }
        }
        let mut node_responses = Vec::new();
        let mut responses: Vec<DynResponse> = Vec::new();
        // Deferred node deletes, applied after the scene releases its `view`
        // borrow (removal must migrate index-keyed state + layout).
        let mut to_delete: Vec<NodeIndex> = Vec::new();
        // Set if a node or the scene makes a CA-affecting edit this pass.
        let mut changed = false;
        let selected: HashSet<egui_graph::NodeId> = state
            .interaction
            .selection
            .nodes
            .iter()
            .map(|ix| egui_graph::NodeId::from_u64(ix.index() as u64))
            .collect();
        let graph_response = self
            .scene_config
            .apply(
                egui_graph::Graph::from_id(self.id)
                    .center_view(do_center)
                    .selected_nodes(selected)
                    .immutable(self.immutable),
            )
            .show(view, ui, |ui, show| {
                let immutable = self.immutable;
                show.nodes(ui, |nctx, ui| {
                    node_responses = nodes(
                        self.registry,
                        self.graph,
                        nctx,
                        state,
                        &mut responses,
                        &mut changed,
                        vm,
                        &mut to_delete,
                        immutable,
                        ui,
                    );
                })
                .edges(ui, |ectx, ui| {
                    edges(self.graph, ectx, state, &mut responses, &mut changed, ui)
                });
            });

        // Sync selection when egui_graph reports a change.
        if let Some(selected) = graph_response.selection_changed {
            state.interaction.selection.nodes = selected
                .into_iter()
                .map(|id| NodeIndex::new(id.value() as usize))
                .collect();
        }

        // Apply deferred deletes now the scene no longer borrows `view`. Removal
        // swap-removes nodes, so this migrates the swapped node's state, layout
        // and selection (see `crate::ops::remove_nodes`). The returned reindex is
        // surfaced so callers can migrate their own index-keyed data too.
        let reindex = crate::ops::remove_nodes(
            self.graph,
            vm,
            &mut view.layout,
            &mut state.interaction.selection,
            to_delete,
        );
        if !reindex.is_empty() {
            changed = true;
        }

        // Track the latest pointer position over the scene (in graph space) so a
        // node added via the command palette lands under the pointer. While the
        // palette window covers the scene, `contains_pointer` is false, so this
        // retains the pre-open position.
        if graph_response.response.contains_pointer() {
            let layer_id = graph_response.response.layer_id;
            let ptr = ui
                .ctx()
                .input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
            if let (Some(ptr), Some(t)) = (ptr, ui.ctx().layer_transform_from_global(layer_id)) {
                state.interaction.last_pointer_pos = Some(t.mul_pos(ptr));
            }
        }

        // Background context menu: graph actions (when mutable) plus a "panes"
        // submenu for toggling pane visibility (available even when immutable).
        let immutable = self.immutable;
        let view_toggles = self.view_toggles;
        let mut reset_layout = false;
        let mut request_layout = false;
        let mut request_center = false;
        if !immutable || view_toggles.is_some() {
            let layer_id = graph_response.response.layer_id;
            graph_response.response.context_menu(|ui| {
                if !immutable {
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
                    if ui
                        .button("auto-layout")
                        .on_hover_text(
                            "lay out the selection, or the whole graph when nothing is selected",
                        )
                        .clicked()
                    {
                        request_layout = true;
                        ui.close();
                    }
                    if ui
                        .button("center-view")
                        .on_hover_text("frame the whole graph in the view")
                        .clicked()
                    {
                        request_center = true;
                        ui.close();
                    }
                }
                if let Some(view) = view_toggles {
                    ui.menu_button("panes", |ui| {
                        crate::widget::panes_config(view, ui);
                        ui.separator();
                        if crate::widget::reset_layout_button(ui) {
                            reset_layout = true;
                            ui.close();
                        }
                    });
                }
            });
        }
        if request_layout {
            state.pending_auto_layout = true;
        }
        if request_center {
            state.pending_center_view = true;
        }
        if reset_layout {
            responses.push(DynResponse::new(ResetTilesLayout));
        }

        // Recover the viewport-independent camera (centre + zoom) from the
        // `egui_graph` view (which `egui` may have panned/zoomed this pass) and
        // write back the possibly-relaid-out node layout.
        scene_view.restore_egui(egui_view, viewport);

        GraphSceneResponse {
            scene: graph_response.response,
            nodes: node_responses,
            changed,
            responses,
            reindex,
        }
    }
}

impl Selection {
    pub fn clear(&mut self) {
        self.edges.clear();
        self.nodes.clear();
    }
}

/// Produce the layout for the given graph (or a `subset` of its nodes).
///
/// The `graph_id` is used to scope node IDs so that nodes with the same index
/// in different graphs don't share egui memory state. When `subset` is `Some`,
/// only those nodes and the edges between them are laid out. The result's
/// bounding box is centred on the origin (see [`apply_auto_layout`] for
/// selection-relative placement).
pub fn layout<N>(
    registry: &dyn Registry,
    graph: &Graph<N>,
    graph_id: egui::Id,
    params: &egui_graph::LayoutParams,
    ctx: &egui::Context,
    subset: Option<&HashSet<NodeIndex>>,
) -> egui_graph::Layout
where
    N: Node,
{
    let included = |n: NodeIndex| subset.is_none_or(|s| s.contains(&n));
    if !graph.node_indices().any(included) {
        return Default::default();
    }
    // Describe each node's sockets (count + padding) and each edge's actual
    // source/destination socket so the (socket-aware) layout can order sockets
    // and minimise edge crossings, matching how the nodes are rendered.
    let get_node = |ca: &gantz_ca::ContentAddr| registry.node(ca);
    let meta_ctx = gantz_core::node::MetaCtx::new(&get_node);
    let socket_padding = egui_graph::socket_padding(&ctx.global_style());
    let nodes_vec = egui_graph::with_graph_memory(ctx, graph_id, |gmem| {
        let node_sizes = gmem.node_sizes();
        graph
            .node_indices()
            .filter(|&n| included(n))
            .map(|n| {
                let node_id = egui_graph::NodeId::from_u64(n.index() as u64);
                let size = node_sizes
                    .get(&node_id)
                    .cloned()
                    .unwrap_or_else(|| [200.0, 50.0].into());
                let node = &graph[n];
                let layout_node = egui_graph::LayoutNode::new(size)
                    .inputs(node.n_inputs(meta_ctx))
                    .outputs(node.n_outputs(meta_ctx))
                    .socket_padding(socket_padding);
                (node_id, layout_node)
            })
            .collect::<Vec<_>>()
    });
    let nodes = nodes_vec.into_iter();
    let edges = graph.edge_indices().filter_map(|e| {
        let (a, b) = graph.edge_endpoints(e)?;
        if !included(a) || !included(b) {
            return None;
        }
        let edge = graph.edge_weight(e)?;
        Some((
            (
                egui_graph::NodeId::from_u64(a.index() as u64),
                edge.output.0 as usize,
            ),
            (
                egui_graph::NodeId::from_u64(b.index() as u64),
                edge.input.0 as usize,
            ),
        ))
    });
    egui_graph::layout(nodes, edges, params.clone())
}

/// Apply a one-shot auto-layout to `view`, laying out `target` and merging the
/// result back into `view.layout`.
///
/// When `whole` (the target is the entire graph), the result is used as-is -
/// centred on the origin. Otherwise the result's bounding box is translated to
/// match the centre of `target`'s current bounding box, so a selection lays out
/// in place rather than snapping toward the origin. Nodes outside `target` are
/// left untouched.
pub fn apply_auto_layout<N>(
    registry: &dyn Registry,
    graph: &Graph<N>,
    graph_id: egui::Id,
    params: &egui_graph::LayoutParams,
    ctx: &egui::Context,
    view: &mut egui_graph::View,
    target: &HashSet<NodeIndex>,
    whole: bool,
) where
    N: Node,
{
    let new = layout(registry, graph, graph_id, params, ctx, Some(target));
    if whole {
        view.layout = new;
        return;
    }
    // Sizes of the target nodes, so bounding boxes use full node rects.
    let sizes: HashMap<egui_graph::NodeId, egui::Vec2> =
        egui_graph::with_graph_memory(ctx, graph_id, |gmem| {
            let node_sizes = gmem.node_sizes();
            target
                .iter()
                .map(|ix| {
                    let id = egui_graph::NodeId::from_u64(ix.index() as u64);
                    let size = node_sizes
                        .get(&id)
                        .copied()
                        .unwrap_or_else(|| [200.0, 50.0].into());
                    (id, size)
                })
                .collect()
        });
    let shift = match (
        bbox_centre(target, &view.layout, &sizes),
        bbox_centre(target, &new, &sizes),
    ) {
        (Some(orig), Some(next)) => orig - next,
        _ => egui::Vec2::ZERO,
    };
    for (id, pos) in new {
        view.layout.insert(id, pos + shift);
    }
}

/// The centre of the bounding box of `target`'s nodes in `layout`, using full
/// node rects (top-left position + size). Returns `None` when no target node
/// has a position in `layout`.
fn bbox_centre(
    target: &HashSet<NodeIndex>,
    layout: &egui_graph::Layout,
    sizes: &HashMap<egui_graph::NodeId, egui::Vec2>,
) -> Option<egui::Pos2> {
    let mut bb: Option<egui::Rect> = None;
    for ix in target {
        let id = egui_graph::NodeId::from_u64(ix.index() as u64);
        let Some(&tl) = layout.get(&id) else { continue };
        let size = sizes
            .get(&id)
            .copied()
            .unwrap_or_else(|| [200.0, 50.0].into());
        let rect = egui::Rect::from_min_size(tl, size);
        bb = Some(bb.map_or(rect, |b| b.union(rect)));
    }
    bb.map(|b| b.center())
}

/// Apply a one-shot align to `view`, moving `target`'s nodes onto a common line
/// via [`egui_graph::align_nodes`]. The feature (`by`) selects the min-edge /
/// centre / max-edge; the orientation (row vs column) is inferred from the
/// selection's spread. Nodes outside `target` are left untouched. The result is
/// written raw - rendering re-snaps every node to the grid.
pub fn apply_align(
    graph_id: egui::Id,
    ctx: &egui::Context,
    view: &mut egui_graph::View,
    target: &HashSet<NodeIndex>,
    by: egui_graph::AlignBy,
) {
    // Node sizes, consulted by `align_nodes` for `AlignBy::Center`/`Max` (a
    // missing node is treated as zero-sized).
    let sizes: HashMap<egui_graph::NodeId, egui::Vec2> =
        egui_graph::with_graph_memory(ctx, graph_id, |gmem| {
            let node_sizes = gmem.node_sizes();
            target
                .iter()
                .map(|ix| {
                    let id = egui_graph::NodeId::from_u64(ix.index() as u64);
                    let size = node_sizes
                        .get(&id)
                        .copied()
                        .unwrap_or_else(|| [200.0, 50.0].into());
                    (id, size)
                })
                .collect()
        });
    let ids = target
        .iter()
        .map(|ix| egui_graph::NodeId::from_u64(ix.index() as u64));
    egui_graph::align_nodes(&mut view.layout, ids, &sizes, by, None);
}

fn nodes<N>(
    registry: &dyn Registry,
    graph: &mut Graph<N>,
    nctx: &mut egui_graph::NodesCtx,
    state: &mut GraphSceneState,
    responses: &mut Vec<DynResponse>,
    changed: &mut bool,
    vm: &mut Engine,
    nodes_to_delete: &mut Vec<NodeIndex>,
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
    let mut nodes_to_reset = Vec::new();
    let mut request_layout = false;
    let mut request_align: Option<egui_graph::AlignBy> = None;
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
                let node_ctx = crate::NodeCtx::new(registry, &node_path, &inlets, &outlets, vm);
                let r = node.ui(node_ctx, nui_ctx);
                *changed |= r.changed;
                responses.extend(r.payloads);
                r.framed
            });

        // Attach on-hover docs to each socket. Each node describes its own
        // sockets (markers read their stored docs; references resolve the
        // referenced graph's marker docs via the registry).
        for (ix, sock) in response.sockets().inputs() {
            if let Some(doc) = node.socket_doc(registry, SocketKind::Input, ix) {
                socket_hover(sock, &doc);
            }
        }
        for (ix, sock) in response.sockets().outputs() {
            if let Some(doc) = node.socket_doc(registry, SocketKind::Output, ix) {
                socket_hover(sock, &doc);
            }
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
                                *changed = true;
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
            // Whether the action target is a multi-node selection (this node is
            // part of a selection of >1). Captured before `target` may be moved.
            let multi = target.len() > 1;
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
            // "open view": detach the right-clicked node's UI into a tile in the
            // Node Views pane (a live mirror, sharing VM state). Always available,
            // even for immutable graphs - it only observes the node.
            if ui
                .button("open view")
                .on_hover_text("open this node's view in the Node Views pane")
                .clicked()
            {
                let ty_name = graph[n_id].name(registry).to_string();
                responses.push(DynResponse::new(OpenNodeView {
                    path: vec![n_ix],
                    ty_name,
                }));
                ui.close();
            }
            if !immutable {
                let stateful = target
                    .iter()
                    .any(|&n| graph.node_weight(n).is_some_and(|w| w.stateful(meta_ctx)));
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
                // Auto-layout the selection (only meaningful for >1 node).
                if multi
                    && ui
                        .button("auto-layout")
                        .on_hover_text("lay out the selected nodes")
                        .clicked()
                {
                    request_layout = true;
                    ui.close();
                }
                // Align the selection onto a common row or column (inferred
                // from the spread). Selection-only, so >1 node.
                if multi {
                    ui.menu_button("align", |ui| {
                        let mut item = |ui: &mut egui::Ui, label, hover, by| {
                            if ui.button(label).on_hover_text(hover).clicked() {
                                request_align = Some(by);
                                ui.close();
                            }
                        };
                        item(
                            ui,
                            "min edges",
                            "align the left (column) or top (row) edges",
                            egui_graph::AlignBy::Min,
                        );
                        item(
                            ui,
                            "centres",
                            "align the node centres",
                            egui_graph::AlignBy::Center,
                        );
                        item(
                            ui,
                            "max edges",
                            "align the right (column) or bottom (row) edges",
                            egui_graph::AlignBy::Max,
                        );
                    });
                }
            }
            // Node-specific items (e.g. the log node's "open logs").
            let node_path = [n_ix];
            let mut node_ctx = crate::NodeCtx::new(registry, &node_path, &inlets, &outlets, vm);
            let cm = graph[n_id].context_menu(&mut node_ctx, ui);
            *changed |= cm.changed;
            responses.extend(cm.payloads);
        });

        node_responses.push((n_id, response));
    }

    // Deletes (keyboard + context menu) are collected into `nodes_to_delete` and
    // applied by the caller via `ops::remove_nodes` once the scene's borrow on
    // the view (and thus its layout) is released - removal must migrate the
    // swapped node's index-keyed state and layout.

    // Reset state by removing it, then re-registering the graph.
    // Registration is idempotent and re-initialises any missing state.
    if !nodes_to_reset.is_empty() {
        for n_id in nodes_to_reset {
            if graph.node_weight(n_id).is_some() {
                let _ = gantz_core::node::state::remove_value(vm, &[n_id.index()]);
            }
        }
        gantz_core::graph::register(&get_node, &*graph, &[], vm);
    }

    // A node-menu "auto-layout"/"align" click is applied on the next pass by
    // `show` (node sizes are then known).
    if request_layout {
        state.pending_auto_layout = true;
    }
    if let Some(by) = request_align {
        state.pending_align = Some(by);
    }

    node_responses
}

/// Show a socket's doc as a hover tooltip (type label in bold, description
/// below).
fn socket_hover(resp: &egui::Response, doc: &SocketDoc) {
    resp.clone().on_hover_ui(|ui| {
        // Re-assert the wrap width every frame. The tooltip's `Area` caches the
        // width it first rendered at (e.g. tiny, just fitting a bare "input"),
        // and wrapped text keeps "fitting" that stale width - so docs added to a
        // referenced graph later never widen it until egui memory is cleared
        // (an app restart). Forcing the max width breaks that feedback loop
        // while still letting short tooltips stay narrow.
        let max_width = ui.spacing().tooltip_width;
        ui.set_max_width(max_width);
        if !doc.ty.is_empty() {
            ui.strong(doc.ty.as_ref());
        }
        if let Some(desc) = &doc.description {
            ui.label(desc.as_ref());
        }
    });
}

fn edges<N>(
    graph: &mut Graph<N>,
    ectx: &mut egui_graph::EdgesCtx,
    state: &mut GraphSceneState,
    responses: &mut Vec<DynResponse>,
    changed: &mut bool,
    ui: &mut egui::Ui,
) {
    // Track whether any edge has a context menu open this frame.
    let mut any_context_menu_open = false;
    // Deferred edge deletes: the loop snapshots edge indices, but `remove_edge`
    // swap-removes, so deleting mid-loop would invalidate a later snapshot index.
    let mut to_delete: Vec<EdgeIndex> = Vec::new();

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
            to_delete.push(e);
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
        let mut delete_edge = false;
        response.context_menu(|ui| {
            if ui.button("inspect").clicked() {
                if let Some(pos) = state.interaction.edge_context_menu_pos.take() {
                    responses.push(DynResponse::new(InspectEdge { edge: e, pos }));
                }
                ui.close();
            }
            if ui.button("delete").clicked() {
                delete_edge = true;
                ui.close();
            }
        });
        // Apply the deletion after the closure releases its borrows. Same effect
        // as the keyboard `deleted()` path above; `changed` propagates to the
        // head's commit so it persists and joins the undo/redo chain.
        if delete_edge {
            graph.remove_edge(e);
            state.interaction.selection.edges.remove(&e);
            *changed = true;
        }
    }

    // Apply deferred edge deletes. `remove_edge` swap-removes (the former-last
    // edge adopts the removed index), so go descending and remap the swapped
    // edge in the selection.
    if !to_delete.is_empty() {
        to_delete.sort_unstable_by_key(|e| std::cmp::Reverse(e.index()));
        to_delete.dedup();
        for e in to_delete {
            if graph.edge_weight(e).is_none() {
                continue;
            }
            let last = graph.edge_count() - 1;
            state.interaction.selection.edges.remove(&e);
            graph.remove_edge(e);
            if e.index() != last {
                let last_e = EdgeIndex::new(last);
                if state.interaction.selection.edges.remove(&last_e) {
                    state.interaction.selection.edges.insert(e);
                }
            }
            *changed = true;
        }
    }

    // Clear the stored position if no context menu is open (user dismissed it).
    if !any_context_menu_open {
        state.interaction.edge_context_menu_pos = None;
    }

    // Draw the in-progress edge if there is one.
    if let Some(edge) = ectx.in_progress(ui) {
        edge.show(ui, egui_graph::bezier::Cubic::DEFAULT_CURVATURE);
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
