//! Rendering of each pane, and the helpers the panes share.

use super::graph_tabs::{Strip, head_scene, tab_strip};
use super::state::{OpenHeadState, SceneConfig, ViewToggles};
use super::tiles::{NodeViewPane, Pane, ext_pane_entries};
use super::{
    FileDrop, FileDropTarget, LogSource, NESTED_GRAPH_TYPE, NodeTyCmd, PaneCtx, WindowStyle,
};
use crate::{
    Action, CopyNodes, CreateNestedGraph, CreateNode, CutNodes, DuplicateNodes, Env,
    ExportAllNamed, ExportHead, ExportPalette, ExportStyle, HeadAccess, ImportPalette, ImportStyle,
    Keymap, NodeCtx, NodeUi, Paste, Redo, ReplaceNode, ResetTilesLayout, Undo,
    node::NodeCodec,
    response::{DynResponse, Responses},
    widget::{self, GraphScene, graph_scene},
};
use gantz_core::node;
use gantz_edit::export;
use petgraph::visit::IntoNodeIdentifiers;
use std::collections::{BTreeSet, HashMap};
use steel::steel_vm::engine::Engine;

/// Render a single pane into `ui`.
///
/// Shared by the tile tree in [`TreeBehaviour::pane_ui`], floating
/// `egui::Window`s and a native host's OS windows, so a pane looks and
/// behaves identically wherever it is shown.
pub(super) fn render_pane<Access>(
    cx: &mut PaneCtx<'_, '_, Access>,
    ui: &mut egui::Ui,
    pane: &mut Pane,
) where
    Access: HeadAccess,
{
    match pane {
        Pane::Ext(key) => render_ext(cx, ui, key),
        Pane::GraphConfig => render_graph_config(cx, ui),
        Pane::GraphScene => render_graph_scene(cx, ui),
        Pane::Graph(id) => render_graph_pane(cx, ui, *id),
        Pane::Graphs => render_graphs(cx, ui),
        Pane::GuiPerf => {
            if let Some(ref mut capture) = cx.gantz.perf_gui {
                perf_view("GUI Perf", capture, ui);
            }
        }
        Pane::History => {
            let heads = cx.access.heads();
            let res = history_view(cx.gantz.env, heads, cx.focused_head, ui);
            push_graph_select(&mut cx.response.responses, res.inner);
        }
        Pane::Logs => render_logs(cx, ui),
        Pane::NodeInspector => render_node_inspector(cx, ui),
        Pane::NodeView(view) => render_node_view(cx, ui, view),
        Pane::Steel => render_steel(cx, ui),
        Pane::GuiPreview => render_gui_preview(cx, ui),
        Pane::GuiTree => render_gui_tree(cx, ui),
        Pane::VmPerf => {
            if let Some(ref mut capture) = cx.gantz.perf_vm {
                perf_view("VM Perf", capture, ui);
            }
        }
        Pane::Settings => render_settings(cx, ui),
    }
}

/// Render the ext pane. See [`render_pane`].
fn render_ext<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui, key: &str)
where
    Access: HeadAccess,
{
    let PaneCtx {
        gantz,
        state,
        access,
        focused_head,
        response: gantz_response,
        ..
    } = cx;
    let focused = access.heads().get(*focused_head).cloned();
    // The focused head's selection as sorted root-level node ids,
    // mirroring the Steel pane's span-highlight input.
    let mut selection: Vec<node::Id> = focused
        .as_ref()
        .and_then(|h| state.open_heads.get(h))
        .map(|hs| {
            hs.scene
                .interaction
                .selection
                .nodes
                .iter()
                .map(|n| n.index())
                .collect()
        })
        .unwrap_or_default();
    selection.sort_unstable();
    match gantz.ext_panes.iter_mut().find(|p| p.key() == key) {
        Some(p) => {
            let cx = widget::ExtPaneCtx {
                focused: focused.as_ref(),
                selection: &selection,
            };
            let res = pane_ui(ui, |ui| p.ui(cx, ui));
            let mut ext_responses = res.inner;
            gantz_response
                .responses
                .extend(focused.as_ref(), ext_responses.drain().map(|(_, d)| d));
        }
        None => {
            ui.centered_and_justified(|ui| {
                ui.weak(format!("'{key}' pane unavailable (no provider)"));
            });
        }
    }
}

/// Render the graph config pane. See [`render_pane`].
fn render_graph_config<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui)
where
    Access: HeadAccess,
{
    let PaneCtx {
        gantz,
        state,
        access,
        focused_head,
        response: gantz_response,
        ..
    } = cx;
    let base_names = gantz.base_names;
    match access.heads().get(*focused_head).cloned() {
        Some(head) => {
            let merge_resolutions = &mut state.merge_resolutions;
            let head_state = state.open_heads.entry(head.clone()).or_default();
            let names = crate::reg::names(gantz.env.registry);
            let is_base = match &head {
                gantz_ca::Head::Branch(name) => base_names.contains_key(name),
                _ => false,
            };
            let immutable = head_immutable(&head, gantz.base_immutable, base_names);

            // Collect the `demo.` names for the dropdown.
            let demo_names: Vec<String> = names
                .iter()
                .filter(|(n, _)| widget::graph_select::is_demo(n))
                .map(|(n, _)| n.to_string())
                .collect();
            let demo_names_vec: Vec<&str> = demo_names.iter().map(|s| s.as_str()).collect();

            // Look up the current demo association for this head.
            let current_demo = match &head {
                gantz_ca::Head::Branch(name) => gantz.env.demo_graph(&name.to_string()),
                _ => None,
            };

            // The graph's current description. Only named graphs have one.
            let current_description = match &head {
                gantz_ca::Head::Branch(name) => {
                    gantz_edit::section::description(gantz.env.registry, name)
                }
                _ => None,
            };

            // The head's session display, when a collab layer is wired.
            let session = gantz.collab.map(|c| match &head {
                gantz_ca::Head::Branch(name) => c.sessions.get(name),
                _ => None,
            });

            let res = pane_ui(ui, |ui| {
                let mut config = widget::GraphConfig::new(&head, head_state, &names)
                    .is_base(is_base)
                    .immutable(immutable)
                    .demo_names(&demo_names_vec)
                    .current_demo(current_demo.as_deref())
                    .current_description(current_description.as_deref())
                    .merge_env(gantz.env, merge_resolutions);
                // The base source dropdown, when authoring context was
                // supplied. See `Gantz::base_sources`.
                if let (Some(ctx), gantz_ca::Head::Branch(name)) = (&gantz.base_sources, &head) {
                    let current = ctx
                        .name_sources
                        .get(&name.to_string())
                        .copied()
                        .unwrap_or(ctx.default_source);
                    config = config.base_sources(ctx.sources, Some(current));
                }
                if let Some(session) = session {
                    config = config.collab(session);
                }
                // A short pane scrolls rather than cut off its last rows.
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| config.show(ui))
                    .inner
            });
            if let Some((original, new_name)) = res.inner.new_branch {
                let branch = crate::BranchHead { original, new_name };
                gantz_response.responses.push(None, branch);
            }
            if let Some(merge) = res.inner.merge {
                gantz_response.responses.push(Some(head.clone()), merge);
            }
            if res.inner.share {
                gantz_response
                    .responses
                    .push(Some(head.clone()), crate::ShareHead { public: true });
            }
            if res.inner.stop_sharing {
                gantz_response
                    .responses
                    .push(Some(head.clone()), crate::StopSharing);
            }
            // Demos, descriptions and base sources belong to named graphs.
            if let gantz_ca::Head::Branch(name) = &head {
                let responses = &mut gantz_response.responses;
                if let Some(demo) = res.inner.demo_changed {
                    let name = name.clone();
                    responses.push(None, crate::SetDemo { name, demo });
                }
                if let Some(description) = res.inner.description_changed {
                    let name = name.clone();
                    responses.push(None, crate::SetDescription { name, description });
                }
                if res.inner.reset_base_graph {
                    responses.push(None, crate::ResetBaseGraph(name.clone()));
                }
                if let Some(source) = res.inner.base_source_changed {
                    let name = name.clone();
                    responses.push(None, crate::SetBaseSource { name, source });
                }
            }
            if res.inner.export {
                gantz_response.responses.push(Some(head), ExportHead);
            }
        }
        None => {
            pane_ui(ui, |ui| {
                ui.label("No graph focused");
            });
        }
    }
}

/// Render the graph scene pane. See [`render_pane`].
fn render_graph_scene<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui)
where
    Access: HeadAccess,
{
    paint_gantz_file_hover_overlay(ui);

    // The rect positions the floating tab strip and sidebar toggle.
    let rect = ui.available_rect_before_wrap();

    // Extension-pane toggle entries for the graph-area context menu.
    let ext_panes = ext_pane_entries(cx.gantz);

    // The main graph fills the scene. The tab strip floats over it.
    let main = cx
        .state
        .graphs
        .resolve_main(cx.access.heads(), cx.focused_head);
    if let Some(main) = &main {
        let id = super::chrome::main_scene_id();
        let bounds = super::chrome::reach_window(ui.ctx(), rect);
        ui.scope_builder(egui::UiBuilder::new().max_rect(bounds), |ui| {
            ui.set_clip_rect(bounds);
            head_scene(cx, ui, main, id, &ext_panes);
        });
    }
    // The scene's tile holds the tree's top-left corner when no pane does,
    // and the strip then leads with the logo, unless the window's header
    // holds it. Patch-only shows the logo alone.
    let tree = super::chrome::tree_bounds(ui.ctx()).unwrap_or(rect);
    let at = |a: egui::Pos2, b: egui::Pos2| (a - b).length() < 0.5;
    let window_style = cx.state.view_toggles.window;
    let window = cx
        .gantz
        .native_window
        .filter(|_| window_style != WindowStyle::Native);
    let header = window.is_some() && window_style == WindowStyle::Header;
    let controls = window.filter(|_| window_style == WindowStyle::Controls);
    let patch_only = cx.state.view_toggles.patch_only;
    let dot = patch_only
        .then(|| super::status::color(super::status::last(ui.ctx()), ui.visuals()))
        .flatten();
    let strip = Strip {
        logo: (at(rect.left_top(), tree.left_top()) && !header).then_some(dot),
        tabs: !patch_only,
        drag: controls.is_some(),
    };
    if strip.logo.is_some() || strip.tabs {
        // The strip sits at the scene tile's corner, so its top meets the side
        // panes' tops and the pane gap sits between them.
        tab_strip(cx, ui, rect.left_top(), main.as_ref(), &ext_panes, strip);
    }
    // The window's controls float over the scene when its tile holds the
    // tree's top-right corner. A pane there holds them in its header.
    if let Some(w) = controls.filter(|_| at(rect.right_top(), tree.right_top())) {
        if let Some(action) = super::decor::floating_controls(ui.ctx(), rect.right_top(), w) {
            cx.response.responses.push(None, action);
        }
    }

    let PaneCtx {
        gantz,
        state,
        access,
        focused_head,
        response: gantz_response,
    } = cx;
    let base_names = gantz.base_names;

    // Show the node palette once rather than per pane, operating on
    // the focused head.
    if let Some(fh) = access.heads().get(*focused_head).cloned() {
        let focused_immutable = head_immutable(&fh, gantz.base_immutable, base_names);

        let head_state = state.open_heads.entry(fh.clone()).or_default();

        // Command keyboard shortcuts, sourced from the keymap.
        if !ui.ctx().egui_wants_keyboard_input() {
            let keymap = &state.keymap;
            // Copy is always allowed.
            if keymap.consume(ui, Action::Copy) {
                let nodes = head_state.scene.interaction.selection.nodes.clone();
                gantz_response
                    .responses
                    .push(Some(fh.clone()), CopyNodes(nodes));
            }
            if keymap.consume(ui, Action::NewGraph) {
                gantz_response.responses.push(None, crate::NewGraph);
            }
            // Paste, undo, redo are gated by immutable.
            if !focused_immutable {
                // Detect paste from an `Event::Paste` or the Paste
                // shortcut. eframe and web send `Event::Paste`.
                // bevy_egui desktop sends `Event::Text` instead.
                let paste_text = ui.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Paste(s) => Some(s.clone()),
                        _ => None,
                    })
                });
                if paste_text.is_some() || keymap.consume(ui, Action::Paste) {
                    let paste = Paste {
                        text: paste_text,
                        pos: crate::PastePos::Offset(egui::vec2(20.0, 20.0)),
                    };
                    gantz_response.responses.push(Some(fh.clone()), paste);
                }
                // Redo before Undo. `consume_shortcut` matches
                // modifiers logically, so `Cmd+Z` also matches a
                // `Cmd+Shift+Z` event. Check and consume the more
                // specific binding first.
                if keymap.consume(ui, Action::Redo) {
                    gantz_response.responses.push(Some(fh.clone()), Redo);
                }
                if keymap.consume(ui, Action::Undo) {
                    gantz_response.responses.push(Some(fh.clone()), Undo);
                }
                // Cut copies the selection, then removes it.
                if keymap.consume(ui, Action::Cut) {
                    let nodes = head_state.scene.interaction.selection.nodes.clone();
                    gantz_response
                        .responses
                        .push(Some(fh.clone()), CutNodes(nodes));
                }
                if keymap.consume(ui, Action::Duplicate) {
                    let nodes = head_state.scene.interaction.selection.nodes.clone();
                    gantz_response
                        .responses
                        .push(Some(fh.clone()), DuplicateNodes(nodes));
                }
            }
        }

        // Skip node palette when immutable.
        if !focused_immutable {
            let editing_name = match &fh {
                gantz_ca::Head::Branch(name) => Some(name.to_string()),
                _ => None,
            };
            let editing = editing_name.as_deref();
            // The pointer position over the focused head's scene in
            // graph coords recorded this frame. New nodes are placed
            // here. It is `Copy`, so no borrow is held across the call.
            let pointer_pos = head_state.scene.interaction.last_pointer_pos;
            // A pending replace applies only while the palette is open
            // over the head that requested it.
            if !state.node_palette.is_visible()
                || state
                    .palette_replace
                    .as_ref()
                    .is_some_and(|(h, _)| h != &fh)
            {
                state.palette_replace = None;
            }
            let replace = state.palette_replace.as_ref().map(|&(_, n)| n);
            let created = node_palette(
                gantz.env,
                editing,
                replace,
                &mut state.node_palette,
                &state.keymap,
                ui,
            );
            match created {
                Some(PaletteChoice::Node(mut create)) => {
                    create.pos = pointer_pos;
                    gantz_response.responses.push(Some(fh), create);
                }
                Some(PaletteChoice::NestedGraph(mut create)) => {
                    create.pos = pointer_pos;
                    gantz_response.responses.push(Some(fh), create);
                }
                Some(PaletteChoice::Replace(replace)) => {
                    state.palette_replace = None;
                    gantz_response.responses.push(Some(fh), replace);
                }
                None => {}
            }
        }
    }
}

/// Render a graph pane: its graph's scene, inset in the pane. See
/// [`render_pane`].
fn render_graph_pane<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui, id: u64)
where
    Access: HeadAccess,
{
    let head = cx.state.graphs.panes.get(&id).cloned();
    let Some(head) = head.filter(|h| cx.access.heads().contains(h)) else {
        ui.centered_and_justified(|ui| ui.weak("this graph is closed"));
        return;
    };
    let ext_panes = ext_pane_entries(cx.gantz);
    // The pane's chrome paints the inset's rounded ground.
    let rect = ui.max_rect().shrink(super::chrome::GRAPH_INSET);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        let scene_id = super::chrome::graph_pane_scene_id(id);
        head_scene(cx, ui, &head, scene_id, &ext_panes);
    });
}

/// Render the graphs pane. See [`render_pane`].
fn render_graphs<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui)
where
    Access: HeadAccess,
{
    let PaneCtx {
        gantz,
        access,
        focused_head,
        response: gantz_response,
        ..
    } = cx;
    let base_names = gantz.base_names;
    // Store the pane rect for file drop targeting.
    ui.ctx().memory_mut(|m| {
        m.data
            .insert_temp(egui::Id::new(GRAPHS_PANE_RECT_ID), ui.max_rect())
    });
    paint_gantz_file_hover_overlay(ui);

    let heads = access.heads();
    let res = graph_select(
        gantz.env,
        heads,
        *focused_head,
        base_names,
        gantz.collab,
        gantz.clipboard,
        ui,
    );
    push_graph_select(&mut gantz_response.responses, res.inner);
}

/// Render the logs pane. See [`render_pane`].
fn render_logs<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui)
where
    Access: HeadAccess,
{
    let PaneCtx {
        gantz,
        state,
        access,
        focused_head,
        ..
    } = cx;
    match &gantz.log_source {
        None => (),
        Some(LogSource::Logger(logger)) => {
            // Resolve labels for entries emitted by nodes of the
            // focused head. The target encodes the node's path.
            let focused = access.heads().get(*focused_head).cloned();
            let mut labels: HashMap<Vec<node::Id>, String> = HashMap::new();
            if let Some(fh) = &focused {
                let paths: BTreeSet<Vec<node::Id>> = logger
                    .get_entries()
                    .iter()
                    .filter_map(|e| gantz_io::log::parse_log_target(&e.target))
                    .collect();
                if !paths.is_empty() {
                    let env = gantz.env;
                    let codec = gantz.codec;
                    access.with_head_mut(fh, |data| {
                        for path in paths {
                            // Log targets are state paths. Only root-level
                            // single-segment ones name a node in this graph.
                            let [ix] = path[..] else { continue };
                            let Some(weight) =
                                data.graph.node_weight(graph_scene::NodeIndex::new(ix))
                            else {
                                continue;
                            };
                            let Ok(inst) = codec.reify(weight) else {
                                continue;
                            };
                            labels.insert(path, inst.node.name(env).to_string());
                        }
                    });
                }
            }
            let res = log_view(logger, &labels, ui);
            // Clicking an entry selects its node. Only root-level nodes
            // live in the focused head, so entries from a nested graph
            // are skipped.
            if let (Some(path), Some(fh)) = (res.inner.clicked_path, focused) {
                if let [node_id] = path[..] {
                    let head_state = state.open_heads.entry(fh.clone()).or_default();
                    let selection = &mut head_state.scene.interaction.selection;
                    selection.clear();
                    selection.nodes.insert(graph_scene::NodeIndex::new(node_id));
                }
            }
        }
        #[cfg(feature = "tracing")]
        Some(LogSource::TraceCapture(trace_capture, level)) => {
            trace_view(trace_capture, *level, ui);
        }
    }
}

/// Render the node inspector pane. See [`render_pane`].
fn render_node_inspector<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui)
where
    Access: HeadAccess,
{
    let PaneCtx {
        gantz,
        state,
        access,
        focused_head,
        response: gantz_response,
        ..
    } = cx;
    let base_names = gantz.base_names;
    if let Some(fh) = access.heads().get(*focused_head).cloned() {
        let immutable = head_immutable(&fh, gantz.base_immutable, base_names);
        let follow_selection = state.follow_selection.inspector;
        let validate = gantz.validates_change_tracking();
        let head_state = state.open_heads.entry(fh.clone()).or_default();
        let ref_ext_uis = gantz.ref_ext_uis;
        let codec = gantz.codec;
        let result = access.with_head_mut(&fh, |data| {
            node_inspector(
                gantz.env,
                codec,
                data.graph,
                data.instances,
                data.vm,
                head_state,
                &fh,
                immutable,
                follow_selection,
                validate,
                ref_ext_uis,
                ui,
            )
            .inner
        });
        if let Some((changed, payloads)) = result {
            if changed {
                gantz_response.changed_heads.push(fh.clone());
            }
            gantz_response.responses.extend(Some(&fh), payloads);
        }
    }
}

/// Render the node view pane. See [`render_pane`].
fn render_node_view<Access>(
    cx: &mut PaneCtx<'_, '_, Access>,
    ui: &mut egui::Ui,
    view: &mut NodeViewPane,
) where
    Access: HeadAccess,
{
    let PaneCtx {
        gantz,
        access,
        response: gantz_response,
        ..
    } = cx;
    // A detached node view renders the node's `view_ui` against its
    // head's live graph and VM, as a mirror sharing state with the
    // in-graph node. A `CentralPanel` gives it the same background as
    // the other panes. No-margin views such as plot drop the pane
    // margin so they fill edge to edge. A placeholder shows when the
    // head is closed.
    let head = view.head.clone();
    let path = view.path.clone();
    let codec = gantz.codec;
    let no_margin = access
        .with_head_mut(&head, |data| {
            let &[ix] = path.as_slice() else {
                return false;
            };
            data.graph
                .node_weight(graph_scene::NodeIndex::new(ix))
                .is_some_and(|w| match data.instances.peek(ix, w) {
                    Some(inst) => inst.node.view_no_margin(),
                    None => codec.reify(w).is_ok_and(|inst| inst.node.view_no_margin()),
                })
        })
        .unwrap_or(false);
    let mut frame = pane_frame(ui.style());
    if no_margin {
        // An edge-to-edge view still keeps clear of the pane's rounded corners.
        let inset = super::chrome::corner_inset(ui.ctx()) as i8;
        frame.inner_margin = egui::Margin::same(inset);
    }
    egui::CentralPanel::default()
        .frame(frame)
        .show_inside(ui, |ui| {
            if !access.heads().iter().any(|h| h == &head) {
                ui.centered_and_justified(|ui| {
                    ui.weak("node's graph is not open");
                });
                return;
            }
            let env = gantz.env;
            let ref_ext_uis = gantz.ref_ext_uis;
            let validate = gantz.validates_change_tracking();
            // VM-state writes recorded by the node's `NodeCtx`.
            let mut writes = Vec::new();
            // Scope child widget ids by head and path so views never
            // share ids with each other or the in-graph node.
            let result = ui
                .push_id((&head, &path), |ui| {
                    access.with_head_mut(&head, |data| {
                        let &[n_ix] = path.as_slice() else {
                            return None;
                        };
                        let (inlets, outlets) = crate::inlet_outlet_ids(env, data.graph);
                        let n_id = graph_scene::NodeIndex::new(n_ix);
                        let weight = data.graph.node_weight(n_id)?;
                        // Panes render sequentially, so each take and
                        // restore pair completes within its site.
                        let mut entry = data.instances.take(codec, n_ix, weight).ok()?;
                        // A locked node's view is read-only. See
                        // `InstanceEntry::lost`.
                        let locked = !entry.lost.is_empty();
                        if locked {
                            widget::locked_label(ui, &entry.lost);
                        }
                        let ctx = NodeCtx::new(
                            env,
                            data.graph,
                            &path,
                            &inlets,
                            &outlets,
                            ref_ext_uis,
                            data.vm,
                            &mut writes,
                        )
                        .read_only(locked);
                        let node = &mut entry.inst.node;
                        let r = ui.add_enabled_ui(!locked, |ui| node.view_ui(ctx, ui)).inner;
                        let weight = &mut data.graph[n_id];
                        let changed = data
                            .instances
                            .restore(n_ix, entry, weight, r.changed, validate);
                        Some((changed, r.payloads))
                    })
                })
                .inner;
            match result {
                Some(Some((changed, payloads))) => {
                    if changed {
                        gantz_response.changed_heads.push(head.clone());
                    }
                    gantz_response.responses.extend(Some(&head), payloads);
                    gantz_response
                        .responses
                        .extend(Some(&head), crate::action::state_written(&mut writes));
                }
                _ => {
                    // Head open but node missing at `path`, for example
                    // removed this frame before migration drops the
                    // view.
                    ui.centered_and_justified(|ui| {
                        ui.weak("node not found");
                    });
                }
            }
        });
}

/// Render the steel pane. See [`render_pane`].
fn render_steel<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui)
where
    Access: HeadAccess,
{
    let PaneCtx {
        state,
        access,
        focused_head,
        ..
    } = cx;
    // Use the focused head's compiled module, highlighting the
    // selected nodes' emitted fns and call sites and any diagnostic
    // spans. A failed compile's error renders above the code.
    let focused = access.heads().get(*focused_head).cloned();
    let compile_error = focused.as_ref().and_then(|h| access.compile_error(h));
    let compiled_steel = focused
        .as_ref()
        .and_then(|h| access.module(h))
        .map(|m| m.src.as_str())
        .unwrap_or("");
    let mut highlights: Vec<std::ops::Range<usize>> = vec![];
    let mut scroll_to = None;
    let mut errors: Vec<std::ops::Range<usize>> = vec![];
    if let Some(h) = &focused {
        errors = access
            .diagnostics(h)
            .iter()
            .filter_map(|d| d.span.clone())
            .collect();
        let head_state = state.open_heads.get(h);
        if let (Some(module), Some(head_state)) = (access.module(h), head_state) {
            let mut selected: Vec<node::Id> = head_state
                .scene
                .interaction
                .selection
                .nodes
                .iter()
                .map(|n| n.index())
                .collect();
            selected.sort_unstable();
            for &ix in &selected {
                // A node at this root level has the single-element
                // path `[ix]` in the compiled module's source map.
                let spans = module.map.node_spans(&[ix]);
                highlights.extend(spans.defs);
                highlights.extend(spans.refs);
            }
            // Scroll to the first highlighted span when the
            // selection changes. The last-seen selection updates
            // even while follow is off, so enabling it does not jump
            // to an old selection.
            let state_id = egui::Id::new("steel_view_selection");
            let current = egui::Id::new(("steel_sel", h, &selected));
            let prev: Option<egui::Id> = ui.ctx().data(|d| d.get_temp(state_id));
            if prev != Some(current) {
                ui.ctx().data_mut(|d| d.insert_temp(state_id, current));
                if state.follow_selection.steel {
                    scroll_to = highlights.iter().map(|r| r.start).min();
                }
            }
        }
    }
    steel_view(
        compiled_steel,
        compile_error,
        &highlights,
        &errors,
        scroll_to,
        ui,
    );
}

/// Render the gui preview pane. See [`render_pane`].
fn render_gui_preview<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui)
where
    Access: HeadAccess,
{
    let PaneCtx {
        gantz,
        access,
        focused_head,
        response: gantz_response,
        ..
    } = cx;
    // The focused graph's own GUI, rendered live against the head's
    // VM. Bindings are correct by construction here, since the tree
    // is this graph's GUI.
    let Some(head) = access.heads().get(*focused_head).cloned() else {
        pane_ui(ui, |ui| ui.weak("no focused graph"));
        return;
    };
    let env = gantz.env;
    let codec = gantz.codec;
    let payloads = pane_ui(ui, |ui| {
        access.with_head_mut(&head, |data| {
            let (role, val) = gui_marker_tree(&head, data.graph, data.vm, ui)?;
            let decoded = gantz_ui::codec::steel::decode(&val, &gantz_ui::Limits::default());
            let payloads = match role {
                // The node-body surfaces preview inside the node
                // chrome, as a reference to this graph shows them.
                crate::node::GuiRole::Body | crate::node::GuiRole::Compact => {
                    gui_preview_node(env, codec, &head, data.graph, data.vm, &decoded, ui)
                }
                crate::node::GuiRole::View | crate::node::GuiRole::Inspector => {
                    egui::ScrollArea::both()
                        .show(ui, |ui| {
                            gui_preview_tree(env, codec, &head, data.graph, data.vm, &decoded, ui)
                        })
                        .inner
                }
            };
            Some(payloads)
        })
    })
    .inner;
    if let Some(payloads) = payloads.flatten() {
        gantz_response.responses.extend(Some(&head), payloads);
    }
}

/// Render the gui tree pane. See [`render_pane`].
fn render_gui_tree<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui)
where
    Access: HeadAccess,
{
    let PaneCtx {
        access,
        focused_head,
        ..
    } = cx;
    // The stored tree of the focused graph's gui marker as scheme
    // text, with its decode warnings below.
    let Some(head) = access.heads().get(*focused_head).cloned() else {
        pane_ui(ui, |ui| ui.weak("no focused graph"));
        return;
    };
    pane_ui(ui, |ui| {
        access.with_head_mut(&head, |data| {
            let Some((_, val)) = gui_marker_tree(&head, data.graph, data.vm, ui) else {
                return;
            };
            let text = gantz_ui::pretty(&gantz_ui::codec::steel::lower(&val), GUI_TREE_WIDTH);
            let decoded = gantz_ui::codec::steel::decode(&val, &gantz_ui::Limits::default());
            egui::ScrollArea::both().show(ui, |ui| {
                widget::SteelView::new(&text).show(ui);
                gui_tree_warnings(&decoded, ui);
            });
        });
    });
}

/// Render the settings pane. See [`render_pane`].
fn render_settings<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui)
where
    Access: HeadAccess,
{
    let PaneCtx {
        gantz,
        state,
        response: gantz_response,
        ..
    } = cx;
    let compile_config = gantz.compile_config;
    let validate_change_tracking = gantz.validate_change_tracking;
    let ext_panes = ext_pane_entries(gantz);
    let ext_tabs = &mut *gantz.settings_tabs;
    let res = pane_ui(ui, |ui| {
        widget::settings(
            &mut state.view_toggles,
            compile_config,
            validate_change_tracking,
            &mut state.layout_config,
            &mut state.scene_config,
            &mut state.follow_selection,
            &mut state.style,
            &mut state.keymap,
            ext_tabs,
            &ext_panes,
            ui,
        )
    });
    if let Some(cfg) = res.inner.compile_config {
        gantz_response.responses.push(None, cfg);
    }
    if let Some(v) = res.inner.validate_change_tracking {
        let validate = crate::ValidateChangeTracking(v);
        gantz_response.responses.push(None, validate);
    }
    if res.inner.reset_all_demos {
        gantz_response.responses.push(None, crate::ResetAllDemos);
    }
    if res.inner.reset_layout {
        gantz_response.responses.push(None, ResetTilesLayout);
    }
    if res.inner.export_style {
        gantz_response.responses.push(None, ExportStyle);
    }
    if res.inner.import_style {
        gantz_response.responses.push(None, ImportStyle);
    }
    if let Some(theme) = res.inner.import_palette {
        gantz_response.responses.push(None, ImportPalette(theme));
    }
    if let Some(palette) = res.inner.export_palette {
        gantz_response.responses.push(None, ExportPalette(palette));
    }
    let mut ext_responses = res.inner.responses;
    gantz_response
        .responses
        .extend(None, ext_responses.drain().map(|(_, d)| d));
}

/// The egui ID used to store the Graphs pane rect for file drop targeting.
const GRAPHS_PANE_RECT_ID: &str = "gantz-graphs-pane-rect";

/// How far past its margin a pane's content may paint. See [`pane_ui`].
const CLIP_BLEED: f32 = 2.0;

/// Paint a hover overlay when `.gantz` files are being dragged over this pane.
///
/// The overlay is best-effort. It only appears when the pointer position is
/// available and within the pane. Some platforms do not track the pointer
/// during OS file drags.
fn paint_gantz_file_hover_overlay(ui: &mut egui::Ui) {
    let rect = ui.max_rect();
    let latest_pos = ui.ctx().input(|i| i.pointer.latest_pos());
    let pointer_over = latest_pos.map(|p| rect.contains(p)).unwrap_or(false);
    let has_hovered = ui.ctx().input(|i| {
        i.raw
            .hovered_files
            .iter()
            .any(|f| export::is_maybe_gantz(f.path.as_deref()))
    });

    if has_hovered && pointer_over {
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            0.0,
            crate::style::color(ui, crate::style::Role::Scrim),
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop to import",
            egui::FontId::proportional(24.0),
            ui.visuals().strong_text_color(),
        );
    }
}

/// Detect `.gantz` file drops from egui's raw input.
///
/// Called from [`Gantz::show`] after the tile tree renders, so that detection
/// is independent of pointer position, which may be unavailable during OS
/// file drags on some platforms. The target pane is Graphs when the pointer
/// is over the stored Graphs pane rect, and [`FileDropTarget::GraphScene`]
/// otherwise.
pub(super) fn collect_gantz_file_drops(ctx: &egui::Context) -> Vec<FileDrop> {
    let dropped = ctx.input(|i| i.raw.dropped_files.clone());
    if dropped.is_empty() {
        return Vec::new();
    }

    let graphs_rect: Option<egui::Rect> =
        ctx.memory(|m| m.data.get_temp(egui::Id::new(GRAPHS_PANE_RECT_ID)));
    let latest_pos = ctx.input(|i| i.pointer.latest_pos());
    let over_graphs = match (graphs_rect, latest_pos) {
        (Some(rect), Some(pos)) => rect.contains(pos),
        _ => false,
    };
    let target = if over_graphs {
        FileDropTarget::Graphs
    } else {
        FileDropTarget::GraphScene
    };

    dropped
        .iter()
        .filter(|f| export::is_maybe_gantz(f.path.as_deref()))
        .filter_map(read_dropped_file)
        .map(|bytes| FileDrop { bytes, target })
        .collect()
}

/// Read bytes from an [`egui::DroppedFile`].
///
/// Tries `file.bytes` first for web, then `std::fs::read` from `file.path` for
/// desktop.
fn read_dropped_file(file: &egui::DroppedFile) -> Option<Vec<u8>> {
    if let Some(ref bytes) = file.bytes {
        return Some(bytes.to_vec());
    }
    if let Some(ref path) = file.path {
        return std::fs::read(path).ok();
    }
    None
}

/// The output count of every node in `g`. It backs the GUI Preview tree's
/// push-eval resolver, since a push entry fn's identity covers the count.
///
/// Nodes reify transiently through the codec. A weight with an unknown tag
/// fails to reify and reports no count.
fn node_output_counts(
    registry: &Env<'_>,
    codec: &crate::node::NodeCodec,
    g: &gantz_ca::DataGraph,
) -> HashMap<node::Id, usize> {
    use gantz_core::Node;
    use petgraph::visit::{IntoNodeReferences, NodeRef};
    let get_node = |ca: &gantz_ca::ContentAddr| registry.node(ca);
    let ctx = gantz_core::node::MetaCtx::new(&get_node);
    g.node_references()
        .filter_map(|n| {
            let inst = codec.reify(n.weight()).ok()?;
            Some((n.id().index(), inst.node.n_outputs(ctx)))
        })
        .collect()
}

/// The column the GUI Tree pane wraps the printed tree at.
const GUI_TREE_WIDTH: usize = 80;

/// The egui memory slot holding the picked `gui` role for `head`. The GUI
/// Preview and GUI Tree panes and their tab badges share it, so one pick
/// drives both panes.
fn gui_role_id(head: &gantz_ca::Head) -> egui::Id {
    egui::Id::new(("gantz-gui-role", head))
}

/// The role the GUI panes show for `head`, given its markers in index
/// order. The picked role when a marker of it exists, else the body, else
/// the first marker's role. `None` when the graph has no marker.
fn picked_gui_role(
    ctx: &egui::Context,
    head: &gantz_ca::Head,
    markers: &[(node::Id, crate::node::Gui)],
) -> Option<crate::node::GuiRole> {
    use crate::node::GuiRole;
    let has = |r: GuiRole| markers.iter().any(|(_, g)| g.role == r);
    ctx.data(|d| d.get_temp::<GuiRole>(gui_role_id(head)))
        .filter(|&r| has(r))
        .or_else(|| has(GuiRole::Body).then_some(GuiRole::Body))
        .or_else(|| markers.first().map(|&(_, g)| g.role))
}

/// Pick the `gui` role the GUI panes show for `head`.
pub(super) fn set_gui_role(ctx: &egui::Context, head: &gantz_ca::Head, role: crate::node::GuiRole) {
    ctx.data_mut(|d| d.insert_temp(gui_role_id(head), role));
}

/// The role badge for a GUI pane's tab, as the focused head, its marker
/// roles in [`GuiRole::ALL`][crate::node::GuiRole::ALL] order and the picked
/// role. `None` unless the head declares at least two roles, since a single
/// role needs no picker.
pub(super) fn gui_role_badge<Access: HeadAccess>(
    access: &mut Access,
    focused_head: usize,
    ctx: &egui::Context,
) -> Option<(
    gantz_ca::Head,
    Vec<crate::node::GuiRole>,
    crate::node::GuiRole,
)> {
    let head = access.heads().get(focused_head).cloned()?;
    let markers = access.with_head_mut(&head, |data| crate::node::gui::markers(data.graph))?;
    let roles: Vec<_> = crate::node::GuiRole::ALL
        .into_iter()
        .filter(|&r| markers.iter().any(|(_, g)| g.role == r))
        .collect();
    if roles.len() < 2 {
        return None;
    }
    let role = picked_gui_role(ctx, &head, &markers)?;
    Some((head, roles, role))
}

/// The picked role and the stored tree of the head's `gui` marker for it.
/// See [`picked_gui_role`].
///
/// `None` when the graph has no marker or the marker has no stored tree
/// yet. Both cases show a hint in place of the content.
fn gui_marker_tree(
    head: &gantz_ca::Head,
    graph: &gantz_ca::DataGraph,
    vm: &Engine,
    ui: &mut egui::Ui,
) -> Option<(crate::node::GuiRole, steel::SteelVal)> {
    let markers = crate::node::gui::markers(graph);
    let Some(role) = picked_gui_role(ui.ctx(), head, &markers) else {
        ui.weak("add a `gui` node to this graph to define its GUI");
        return None;
    };
    // First marker of the role, in index order.
    let &(ix, _) = markers
        .iter()
        .find(|(_, g)| g.role == role)
        .expect("role picked from existing markers");
    let val = node::state::extract_value(vm, &[ix])
        .ok()
        .flatten()
        .filter(|v| !matches!(v, steel::SteelVal::Void));
    if val.is_none() {
        ui.weak("the marker has no stored tree yet");
    }
    Some((role, val?))
}

/// Render a decoded body tree inside the node chrome a reference to `head`
/// has in a scene: the node frame with the graph's inlet and outlet sockets
/// and their docs. The node is static and centred in the pane. The returned
/// payloads are the tree's, see [`gui_preview_tree`].
fn gui_preview_node(
    env: &Env<'_>,
    codec: &crate::node::NodeCodec,
    head: &gantz_ca::Head,
    graph: &gantz_ca::DataGraph,
    vm: &mut Engine,
    decoded: &gantz_ui::Decoded,
    ui: &mut egui::Ui,
) -> Vec<crate::response::DynResponse> {
    let (inlets, outlets) = crate::inlet_outlet_ids(env, graph);
    let head_ca: Option<gantz_ca::ContentAddr> = env
        .registry
        .head_commit(head)
        .map(|commit| commit.graph.into());

    // Centre on the size measured last frame. The first frame draws at the
    // top left.
    let size_id = egui::Id::new(("gantz-gui-preview-node-size", head));
    let last_size: egui::Vec2 = ui
        .ctx()
        .data(|d| d.get_temp(size_id))
        .unwrap_or(egui::Vec2::ZERO);
    let avail = ui.available_rect_before_wrap();
    let offset = ((avail.size() - last_size) * 0.5).max(egui::Vec2::ZERO);
    let rect = egui::Rect::from_min_size(avail.min + offset, avail.size() - offset);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));

    let resp = egui_graph::node::Node::from_id(egui_graph::NodeId(0))
        .inputs(inlets.len())
        .outputs(outlets.len())
        .flow(egui::Direction::TopDown)
        .max_width(f32::INFINITY)
        .show_static(&mut child, |uictx| {
            uictx.framed(|ui, _sockets| gui_preview_tree(env, codec, head, graph, vm, decoded, ui))
        });
    ui.ctx()
        .data_mut(|d| d.insert_temp(size_id, resp.inner.response.rect.size()));

    // The sockets carry the graph's marker docs, as they do on a reference
    // node in a scene.
    if let Some(ca) = head_ca {
        for (ix, sock) in resp.sockets.inputs() {
            if let Some(doc) = env.socket_doc(&ca, crate::SocketKind::Input, ix) {
                super::graph_scene::socket_hover(sock, &doc);
            }
        }
        for (ix, sock) in resp.sockets.outputs() {
            if let Some(doc) = env.socket_doc(&ca, crate::SocketKind::Output, ix) {
                super::graph_scene::socket_hover(sock, &doc);
            }
        }
    }
    resp.inner.inner
}

/// Render a decoded tree against a head's live VM. This is the GUI Preview
/// pane's body. Bindings resolve into the head's node state both ways. The
/// returned payloads carry the tree's push evaluations and state writes.
fn gui_preview_tree(
    env: &Env<'_>,
    codec: &crate::node::NodeCodec,
    head: &gantz_ca::Head,
    graph: &gantz_ca::DataGraph,
    vm: &mut Engine,
    decoded: &gantz_ui::Decoded,
    ui: &mut egui::Ui,
) -> Vec<crate::response::DynResponse> {
    let (inlets, outlets) = crate::inlet_outlet_ids(env, graph);
    // The focused head's committed graph address. Resolver hops into
    // instances go through the registry, since instances always resolve
    // committed children.
    let head_ca: Option<gantz_ca::ContentAddr> = env
        .registry
        .head_commit(head)
        .map(|commit| commit.graph.into());
    let n_outs = node_output_counts(env, codec, graph);
    let resolver = |p: &[node::Id]| -> Option<usize> {
        match p {
            // Top-level nodes read the working graph.
            [ix] => n_outs.get(ix).copied(),
            _ => crate::reg::n_outputs_at(env, head_ca.as_ref()?, p),
        }
    };
    let ref_gui = |chain: &[node::Id]| -> Option<node::Id> {
        let (_, marker) = crate::reg::resolve_ref_chain(env, head_ca?, chain)?;
        Some(marker)
    };
    let mut writes = Vec::new();
    let mut node_ctx = NodeCtx::new(env, graph, &[], &inlets, &outlets, &[], vm, &mut writes);
    let root_id = egui::Id::new(("gantz-gui-preview", head));
    let r = crate::ui_tree::UiTree::new(root_id)
        .n_outputs(&resolver)
        .ref_gui(&ref_gui)
        .show(&decoded.root, &mut node_ctx, ui);
    let mut payloads = r.payloads;
    payloads.extend(
        writes
            .drain(..)
            .map(|w| crate::DynResponse::new(crate::StateWritten(w))),
    );
    payloads
}

/// The GUI Tree pane's collapsible decode-warnings list.
fn gui_tree_warnings(decoded: &gantz_ui::Decoded, ui: &mut egui::Ui) {
    if decoded.warnings.is_empty() {
        return;
    }
    let title = format!("warnings ({})", decoded.warnings.len());
    egui::CollapsingHeader::new(title).show(ui, |ui| {
        for w in &decoded.warnings {
            ui.horizontal_wrapped(|ui| {
                ui.weak(format!("{:?}", w.path.0));
                ui.label(w.kind.to_string());
            });
        }
    });
}

/// Provides a consistent frame and styling for the panes.
pub(super) fn pane_ui<R>(
    ui: &mut egui::Ui,
    pane: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::CentralPanel::default()
        .frame(pane_frame(ui.style()))
        .show_inside(ui, |ui| {
            // Content keeps to the pane's margin. The small bleed leaves room
            // for the outline of a field that fills the width.
            let clip = ui.max_rect().expand(CLIP_BLEED).intersect(ui.clip_rect());
            ui.set_clip_rect(clip);
            pane(ui)
        })
}

/// The frame of a pane's body. It has no fill, since the pane's rounded
/// ground is painted beneath the tree. See `chrome`.
pub(super) fn pane_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::central_panel(style)
        .fill(egui::Color32::TRANSPARENT)
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 0,
            bottom: 10,
        })
}

fn graph_select(
    env: &Env<'_>,
    heads: &[gantz_ca::Head],
    focused_head: usize,
    base_names: &crate::reg::Names,
    collab: Option<&crate::collab::CollabUiState>,
    clipboard: Option<&dyn Fn() -> Option<String>>,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<widget::graph_select::GraphSelectResponse> {
    pane_ui(ui, |ui| {
        widget::GraphSelect::new(env, heads, base_names)
            .focused_head(focused_head)
            .collab(collab)
            .clipboard(clipboard)
            .show(ui)
    })
}

fn history_view(
    env: &Env<'_>,
    heads: &[gantz_ca::Head],
    focused_head: usize,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<widget::graph_select::GraphSelectResponse> {
    pane_ui(ui, |ui| {
        widget::HistoryView::new(env, heads)
            .focused_head(focused_head)
            .show(ui)
    })
}

/// Push the actions of a Graphs or History pane response as payloads.
fn push_graph_select(responses: &mut Responses, res: widget::graph_select::GraphSelectResponse) {
    let widget::graph_select::GraphSelectResponse {
        new_graph,
        import,
        export_all,
        replaced,
        opened,
        closed,
        name_removed,
        join_ticket,
    } = res;
    if new_graph {
        responses.push(None, crate::NewGraph);
    }
    if import {
        responses.push(None, crate::Import);
    }
    if export_all {
        responses.push(None, ExportAllNamed);
    }
    if let Some(head) = replaced {
        responses.push(None, crate::ReplaceHead(head));
    }
    if let Some(head) = opened {
        responses.push(None, crate::OpenHead(head));
    }
    if let Some(head) = closed {
        responses.push(None, crate::CloseHead(head));
    }
    if let Some(name) = name_removed {
        responses.push(None, crate::RemoveName(name));
    }
    if let Some(ticket) = join_ticket {
        responses.push(None, crate::JoinSession { ticket });
    }
}

fn perf_view(title: &str, capture: &mut widget::PerfCapture, ui: &mut egui::Ui) {
    pane_ui(ui, |ui| widget::PerfView::new(title, capture).show(ui));
}

/// Returns the response from the graph scene if it was shown.
///
/// Payloads emitted within the scene are returned in
/// [`GraphSceneResponse::responses`][graph_scene::GraphSceneResponse] for the
/// caller to tag and merge. `id` is the scene's egui id. It must stay the
/// same while the head moves, as on a commit to a detached head, so the scene's
/// egui state and layers persist.
#[allow(clippy::too_many_arguments)]
pub(super) fn graph_scene(
    id: egui::Id,
    registry: &Env<'_>,
    codec: &NodeCodec,
    graph: &mut gantz_ca::DataGraph,
    instances: &mut crate::node::NodeInstances,
    head: &gantz_ca::Head,
    head_state: &mut OpenHeadState,
    view_toggles: &mut ViewToggles,
    ext_panes: &[widget::ExtPaneEntry],
    edge_styles: &[&dyn widget::EdgeStyle],
    head_view: &mut gantz_edit::SceneView,
    layout_params: egui_graph::LayoutParams,
    scene_config: SceneConfig,
    immutable: bool,
    validate_change_tracking: bool,
    keymap: &Keymap,
    diagnostics: &[gantz_core::Diagnostic],
    vm: &mut Engine,
    ui: &mut egui::Ui,
) -> Option<graph_scene::GraphSceneResponse> {
    // Select-all replaces the selection with every node. Handled here rather
    // than the outer command block because this is where the graph is in
    // scope. Gated like other command shortcuts so it does not fire while
    // typing.
    if !ui.ctx().egui_wants_keyboard_input() && keymap.consume(ui, Action::SelectAll) {
        head_state.scene.interaction.selection.nodes = graph.node_indices().collect();
        head_state.scene.interaction.selection.edges.clear();
    }

    // Seed the node layout the first time this graph is shown, and centre the
    // camera on it at zoom 1. Without an explicit camera the scene would fall
    // back to egui's fit-to-bounds, zooming out to frame every node. A freshly
    // opened graph should instead sit at its natural 1:1 zoom.
    if head_view.layout.is_empty() {
        head_view.layout = crate::view::from_egui_layout(widget::graph_scene::layout(
            registry,
            codec,
            graph,
            id,
            &layout_params,
            ui.ctx(),
            None,
        ));
        let mut bounds: Option<egui::Rect> = None;
        for &pos in head_view.layout.values() {
            let r = egui::Rect::from_min_size(pos, egui::Vec2::ZERO);
            bounds = Some(bounds.map_or(r, |b| b.union(r)));
        }
        head_view.camera = gantz_edit::Camera {
            center: bounds.map_or(egui::Pos2::ZERO, |b| b.center()),
            zoom: 1.0,
        };
    }

    let response = GraphScene::new(registry, codec, graph, instances)
        .with_id(id)
        .layout_params(layout_params)
        .scene_config(scene_config)
        .immutable(immutable)
        .validate_change_tracking(validate_change_tracking)
        .view_toggles(view_toggles)
        .ext_panes(ext_panes)
        .edge_styles(head, edge_styles)
        .show(head_view, &mut head_state.scene, vm, ui);

    graph_scene::paint_diagnostics(diagnostics, &[], &response, ui);

    Some(response)
}

/// A node choice made in the node palette.
pub(super) enum PaletteChoice {
    /// Create an ordinary node of the given type.
    Node(CreateNode),
    /// Create a new nested graph via the reserved [`NESTED_GRAPH_TYPE`] entry.
    NestedGraph(CreateNestedGraph),
    /// Replace an existing node with a node of the chosen type.
    Replace(ReplaceNode),
}

/// Returns a node-creation payload when a node type is chosen, or a replace
/// payload when `replace` holds the node the palette replaces.
///
/// `editing` is the focused head's name when it is a branch. It hides node
/// types whose reference would cycle back to the graph being edited.
pub(super) fn node_palette(
    env: &Env<'_>,
    editing: Option<&str>,
    replace: Option<graph_scene::NodeIndex>,
    node_palette: &mut widget::NodePalette,
    keymap: &Keymap,
    ui: &mut egui::Ui,
) -> Option<PaletteChoice> {
    // Toggle node palette visibility via its keymap binding.
    if !ui.ctx().egui_wants_keyboard_input() && keymap.consume(ui, Action::ToggleNodePalette) {
        node_palette.toggle();
    }

    // Map the node types to commands for the node palette, dropping any type
    // whose reference would form a cycle back to the editing graph. The reserved
    // nested-graph entry always mints a fresh child, so it is never cyclic. A
    // replace hides it, since a fresh nested graph has no sockets to keep.
    let types: Vec<&str> = env
        .node_types()
        .into_iter()
        .filter(|&k| match k == NESTED_GRAPH_TYPE {
            true => replace.is_none(),
            false => editing.is_none_or(|e| !env.would_ref_cycle(k, e)),
        })
        .collect();
    let cmds = types.iter().map(|&k| NodeTyCmd { env, name: k });

    // The chosen node type becomes a creation or replace payload. The reserved
    // `NESTED_GRAPH_TYPE` routes to the registry-aware nested-graph op. The
    // palette is centered over the graph scene, which is this `ui`'s rect.
    let scene_rect = ui.max_rect();
    node_palette.show(ui.ctx(), scene_rect, cmds).map(|cmd| {
        // The placement position is filled in by the caller, which has access to
        // the focused head's last pointer position.
        if let Some(node) = replace {
            PaletteChoice::Replace(ReplaceNode {
                node,
                node_type: cmd.name.to_string(),
            })
        } else if cmd.name == NESTED_GRAPH_TYPE {
            PaletteChoice::NestedGraph(CreateNestedGraph { pos: None })
        } else {
            PaletteChoice::Node(CreateNode {
                node_type: cmd.name.to_string(),
                pos: None,
            })
        }
    })
}

pub(super) fn log_view(
    logger: &widget::log_view::Logger,
    node_labels: &HashMap<Vec<node::Id>, String>,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<widget::log_view::LogViewResponse> {
    pane_ui(ui, |ui| {
        widget::log_view::LogView::new("log-view".into(), logger.clone())
            .node_labels(node_labels)
            .show(ui)
    })
}

#[cfg(feature = "tracing")]
pub(super) fn trace_view(
    trace_capture: &widget::trace_view::TraceCapture,
    level: tracing::level_filters::LevelFilter,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<()> {
    pane_ui(ui, |ui| {
        widget::trace_view::TraceView::new("trace-view".into(), trace_capture.clone(), level)
            .show(ui);
    })
}

/// Whether the given head should be treated as immutable.
///
/// A head is immutable when `base_immutable` is enabled and the head is a base
/// graph that is not a demo. Demo base graphs are always mutable so users can
/// experiment.
pub(super) fn head_immutable(
    head: &gantz_ca::Head,
    base_immutable: bool,
    base_names: &crate::reg::Names,
) -> bool {
    let is_base = matches!(head, gantz_ca::Head::Branch(name) if base_names.contains_key(name));
    let is_demo =
        matches!(head, gantz_ca::Head::Branch(name) if widget::graph_select::is_demo(name));
    base_immutable && is_base && !is_demo
}

/// Returns whether any inspected node had a CA-affecting edit, together with
/// the payloads emitted by node UIs within the inspector.
///
/// `follow_selection` scrolls to the first selected node when the selection
/// changes. `validate` turns on change-tracking validation. See
/// [`crate::node::NodeInstances::restore`].
#[allow(clippy::too_many_arguments)]
pub(super) fn node_inspector<'a>(
    registry: &'a Env<'a>,
    codec: &NodeCodec,
    root: &mut gantz_ca::DataGraph,
    instances: &mut crate::node::NodeInstances,
    vm: &mut Engine,
    head_state: &mut OpenHeadState,
    head: &gantz_ca::Head,
    immutable: bool,
    follow_selection: bool,
    validate: bool,
    ref_ext_uis: &'a [&'a dyn crate::node::RefExtUi],
    ui: &mut egui::Ui,
) -> egui::InnerResponse<(bool, Vec<DynResponse>)> {
    pane_ui(ui, |ui| {
        let mut responses = Vec::new();
        let mut changed = false;
        egui::ScrollArea::vertical()
            .auto_shrink(egui::Vec2b::FALSE)
            .show(ui, |ui| {
                let graph = &mut *root;
                let ids: Vec<_> = graph.node_identifiers().collect();
                let (inlets, outlets) = crate::inlet_outlet_ids(registry, graph);
                // The rect of the first selected node, used to scroll to it.
                let mut selected_rect: Option<egui::Rect> = None;
                // VM-state writes recorded by each node's `NodeCtx`. They
                // drain per node into `StateWritten` payloads.
                let mut writes = Vec::new();
                for id in ids {
                    let is_selected = head_state.scene.interaction.selection.nodes.contains(&id);
                    let frame = widget::kit::card_frame(ui, is_selected);
                    // Each node gets its own id scope, so its table, grids and
                    // popups keep their state apart from the other nodes.
                    let frame_resp = ui
                        .push_id(id, |ui| {
                            frame.show(ui, |ui| {
                                let Some(weight) = graph.node_weight(id) else {
                                    return;
                                };
                                let ix = id.index();
                                // An unknown tag shows a weak placeholder row.
                                let Ok(mut entry) = instances.take(codec, ix, weight) else {
                                    ui.weak(format!("{} (unknown node type)", weight.tag));
                                    return;
                                };
                                // A locked node shows why above its disabled rows. See
                                // `InstanceEntry::lost`.
                                let locked = !entry.lost.is_empty();
                                if locked {
                                    widget::locked_label(ui, &entry.lost);
                                    if !immutable && widget::discard_button(ui) {
                                        let drop = crate::DropUnknownData { node: id };
                                        responses.push(DynResponse::new(drop));
                                    }
                                }
                                let path = [ix];
                                let ctx = NodeCtx::new(
                                    registry,
                                    graph,
                                    &path[..],
                                    &inlets,
                                    &outlets,
                                    ref_ext_uis,
                                    vm,
                                    &mut writes,
                                )
                                .read_only(locked);
                                let node = &mut entry.inst.node;
                                let resp =
                                    widget::NodeInspector::new(node, ctx, immutable || locked)
                                        .show(ui);
                                let weight = &mut graph[id];
                                changed |=
                                    instances.restore(ix, entry, weight, resp.changed, validate);
                                responses.extend(resp.payloads);
                                if resp.label_response.clicked() {
                                    let sel = &mut head_state.scene.interaction.selection.nodes;
                                    if ui.input(|i| i.modifiers.command) {
                                        if !sel.remove(&id) {
                                            sel.insert(id);
                                        }
                                    } else {
                                        sel.clear();
                                        sel.insert(id);
                                    }
                                }
                            })
                        })
                        .inner;
                    responses.extend(crate::action::state_written(&mut writes));
                    if is_selected && selected_rect.is_none() {
                        selected_rect = Some(frame_resp.response.rect);
                    }
                }

                // Scroll to the first selected node when the selection changes,
                // mirroring the Steel view's scroll-to-span on selection. The
                // last-seen selection updates even while follow is off, so
                // enabling it does not jump to an old selection.
                let state_id = egui::Id::new("node_inspector_selection");
                let mut selected: Vec<node::Id> = head_state
                    .scene
                    .interaction
                    .selection
                    .nodes
                    .iter()
                    .map(|n| n.index())
                    .collect();
                selected.sort_unstable();
                let current = egui::Id::new(("inspector_sel", head, &selected));
                let prev: Option<egui::Id> = ui.ctx().data(|d| d.get_temp(state_id));
                if prev != Some(current) {
                    ui.ctx().data_mut(|d| d.insert_temp(state_id, current));
                    if let Some(rect) = selected_rect.filter(|_| follow_selection) {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                }
            });
        (changed, responses)
    })
}

fn steel_view(
    compiled_steel: &str,
    compile_error: Option<&str>,
    highlights: &[std::ops::Range<usize>],
    errors: &[std::ops::Range<usize>],
    scroll_to: Option<usize>,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<()> {
    pane_ui(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink(egui::Vec2b::FALSE)
            .show(ui, |ui| {
                if let Some(error) = compile_error {
                    let color = ui.visuals().error_fg_color;
                    let text = egui::RichText::new(error).monospace().color(color);
                    ui.add(egui::Label::new(text).selectable(true));
                    if !compiled_steel.is_empty() {
                        ui.separator();
                    }
                }
                widget::SteelView::new(compiled_steel)
                    .highlights(highlights)
                    .errors(errors)
                    .scroll_to(scroll_to)
                    .show(ui);
            });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owned parts of an [`Env`] over an empty registry.
    struct EnvParts {
        registry: gantz_ca::Registry,
        graphs: gantz_core::data::ReifiedGraphs<crate::node::DynNode>,
        builtins: gantz_core::Builtins,
        ui_builtins: crate::node::UiBuiltins,
        codec: NodeCodec,
    }

    impl EnvParts {
        fn new() -> Self {
            Self {
                registry: gantz_ca::Registry::default(),
                graphs: gantz_core::data::ReifiedGraphs::new(),
                builtins: gantz_core::Builtins::default(),
                ui_builtins: crate::node::UiBuiltins::default(),
                codec: crate::test_node::codec(),
            }
        }

        fn env(&self) -> Env<'_> {
            Env {
                registry: &self.registry,
                builtins: &self.builtins,
                codec: &self.codec,
                graphs: &self.graphs,
                instances: &self.ui_builtins,
            }
        }
    }

    /// A graph of two unconnected `expr` nodes.
    fn two_exprs() -> gantz_ca::DataGraph {
        let expr = |src| {
            let node = gantz_core::node::Expr::new(src).unwrap();
            gantz_core::data::erase_node_typed(&node).unwrap()
        };
        let mut graph = gantz_ca::DataGraph::default();
        graph.add_node(expr("(+ 1 2)"));
        graph.add_node(expr("(+ 3 4)"));
        graph
    }

    /// The scene keeps its egui state under the caller's id, whatever the
    /// head. A head that moves, as on a commit to a detached head, keeps its
    /// scene state.
    #[test]
    fn the_scene_state_does_not_follow_the_head() {
        let parts = EnvParts::new();
        let env = parts.env();
        let mut graph = two_exprs();
        let mut instances = crate::node::NodeInstances::default();
        let mut vm = Engine::new_base();
        let mut head_state = OpenHeadState::default();
        let mut view_toggles = ViewToggles::default();
        let mut head_view = gantz_edit::SceneView::default();
        let keymap = Keymap::default();
        let ctx = egui::Context::default();
        let id = egui::Id::new("scene");
        let commit = |b| gantz_ca::Head::Commit(gantz_ca::ContentAddr::from([b; 32]).into());
        let (a, b) = (commit(1), commit(2));
        for head in [&a, &b] {
            let _ = ctx.run_ui(Default::default(), |ui| {
                graph_scene(
                    id,
                    &env,
                    &parts.codec,
                    &mut graph,
                    &mut instances,
                    head,
                    &mut head_state,
                    &mut view_toggles,
                    &[],
                    &[],
                    &mut head_view,
                    widget::LayoutConfig::default().to_params(egui::Direction::TopDown),
                    Default::default(),
                    false,
                    false,
                    &keymap,
                    &[],
                    &mut vm,
                    ui,
                );
            });
        }
        let sizes = |id| egui_graph::with_graph_memory(&ctx, id, |m| m.node_sizes().len());
        assert_eq!(sizes(id), 2);
        assert_eq!(sizes(egui::Id::new(&a)), 0);
        assert_eq!(sizes(egui::Id::new(&b)), 0);
    }

    /// Every action of a Graphs or History pane response becomes a payload.
    #[test]
    fn graph_select_actions_become_payloads() {
        let name: gantz_ca::Name = "jam".parse().unwrap();
        let head = gantz_ca::Head::Branch(name.clone());
        let res = widget::graph_select::GraphSelectResponse {
            new_graph: true,
            import: true,
            export_all: true,
            replaced: Some(head.clone()),
            opened: Some(head.clone()),
            closed: Some(head.clone()),
            name_removed: Some(name),
            join_ticket: Some("ticket".to_string()),
        };
        let mut responses = Responses::default();
        push_graph_select(&mut responses, res);
        assert_eq!(responses.take::<crate::NewGraph>().len(), 1);
        assert_eq!(responses.take::<crate::Import>().len(), 1);
        assert_eq!(responses.take::<ExportAllNamed>().len(), 1);
        assert_eq!(responses.take::<crate::ReplaceHead>().len(), 1);
        assert_eq!(responses.take::<crate::OpenHead>().len(), 1);
        assert_eq!(responses.take::<crate::CloseHead>().len(), 1);
        assert_eq!(responses.take::<crate::RemoveName>().len(), 1);
        assert_eq!(responses.take::<crate::JoinSession>().len(), 1);
        assert!(responses.is_empty());
    }

    /// The text of every shape painted in `shape`, nested shapes included.
    fn shape_texts(shape: &egui::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => texts.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| shape_texts(s, texts)),
            _ => (),
        }
    }

    /// Each inspected node draws its table in its own id scope. egui paints a
    /// "use of ... ID" warning over widgets whose ids clash, such as the
    /// column resize handles of two tables with one id.
    #[test]
    fn inspected_nodes_have_their_own_ids() {
        let parts = EnvParts::new();
        let env = parts.env();
        let codec = &parts.codec;
        let mut graph = two_exprs();
        let mut instances = crate::node::NodeInstances::default();
        let mut vm = Engine::new_base();
        let mut head_state = OpenHeadState::default();
        let head = gantz_ca::Head::Branch("test".parse().unwrap());

        let ctx = egui::Context::default();
        assert!(ctx.options(|o| o.warn_on_id_clash));
        let mut texts = Vec::new();
        // Tables size their columns over the first passes.
        for _ in 0..3 {
            let output = ctx.run_ui(Default::default(), |ui| {
                node_inspector(
                    &env,
                    codec,
                    &mut graph,
                    &mut instances,
                    &mut vm,
                    &mut head_state,
                    &head,
                    false,
                    false,
                    true,
                    &[],
                    ui,
                );
            });
            texts.clear();
            for clipped in &output.shapes {
                shape_texts(&clipped.shape, &mut texts);
            }
        }
        assert!(texts.iter().any(|t| t == "path"), "the tables drew");
        let clashes: Vec<_> = texts.iter().filter(|t| t.contains(" ID ")).collect();
        assert!(clashes.is_empty(), "{clashes:?}");
    }
}
