//! A Plot node for visualising numeric values flowing through the graph.
//!
//! The node body is intentionally minimal - just a plot - while its appearance
//! and behaviour are configured through the node inspector and context menu.
//!
//! Two modes are supported:
//! - [`PlotMode::Scope`]: accumulate a bounded, scrolling history and plot it
//!   like an oscilloscope. Each pushed number is appended; a pushed list or vector
//!   extends the history with its numeric elements; a pushed list of channels (see
//!   below) accumulates one history per channel.
//! - [`PlotMode::Signal`]: plot the incoming value directly (a list or vector as a
//!   series, a single number as one bar), replacing it on each evaluation.
//!
//! In both modes a list or vector *of lists/vectors* (a list of channels, e.g.
//! `~scopeout`'s per-channel rings) is drawn as one stacked sub-plot per channel.
//!
//! Steel lists ([`SteelVal::ListV`]) and vectors ([`SteelVal::VectorV`]) are accepted
//! interchangeably throughout; the scope history is stored as a vector.
//!
//! In both modes the node is a pass-through: its output forwards the input
//! value unchanged (like [`super::Inspect`]), so a value can be observed without
//! breaking the chain it flows through.

use super::size_sync::{self, fitted_size};
use crate::ui_tree::UiTree;
use crate::ui_tree::plot::{is_container, resolve_color, split_channels};
use crate::widget::node_inspector;
use crate::widget::node_inspector::radio_option;
use crate::{
    ContextMenuResponse, Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse,
    NodeViewResponse, SocketDoc, SocketKind,
};
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use steel::gc::Gc;
use steel::steel_vm::register_fn::RegisterFn;
use steel::{SteelVal, Vector};

/// An `f32` that participates in `Hash` via its bit pattern, letting
/// float-valued config keep [`Plot`]'s `Hash` derive (`f32` is not `Hash`).
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct F32(pub f32);

impl F32 {
    fn get(self) -> f32 {
        self.0
    }
}

impl std::hash::Hash for F32 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.0.to_bits(), state);
    }
}

/// How the plot interprets and accumulates its input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub enum PlotMode {
    /// Accumulate a bounded scrolling history (numbers appended, lists extend).
    Scope,
    /// Plot the incoming value directly, replacing the prior.
    Signal,
}

/// How the series is drawn.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub enum PlotStyle {
    /// Contiguous bars (the default).
    Bars,
    /// A connected line.
    Line,
}

/// A node that plots the numeric values it receives.
///
/// Every field feeds the content address (none are `#[serde(skip)]`), so each
/// inspector edit is a real, persisted, undoable change rather than transient
/// view state.
#[derive(Clone, Debug, Hash, Deserialize, Serialize, NodeTag)]
pub struct Plot {
    /// Scope (scalar history) or Signal (plot the value directly).
    mode: PlotMode,
    /// Bars or line.
    style: PlotStyle,
    /// The maximum number of samples retained in [`PlotMode::Scope`].
    capacity: u32,
    /// Persisted body width.
    width: u16,
    /// Persisted body height.
    height: u16,
    /// Line/bar colour. `None` follows the theme's strong text colour.
    color: Option<[u8; 4]>,
    /// Whether to draw the background grid.
    show_grid: bool,
    /// Whether to draw the axes.
    show_axes: bool,
    /// When on, hovering shows a crosshair and the value beneath it. The plot
    /// never pans or zooms regardless - the node drags and right-clicks as usual.
    interactive: bool,
    /// When on, the plot is inset within the node frame's regular margin; when
    /// off the data fills the frame.
    margin: bool,
    /// A fixed lower bound for the value axis when `Some`.
    y_min: Option<F32>,
    /// A fixed upper bound for the value axis when `Some`.
    y_max: Option<F32>,
}

impl Plot {
    /// The default body size, `[width, height]`.
    pub const DEFAULT_SIZE: [u16; 2] = [120, 80];
    /// The default scope history capacity.
    pub const DEFAULT_CAPACITY: u32 = 256;
}

impl Default for Plot {
    fn default() -> Self {
        Self {
            mode: PlotMode::Scope,
            style: PlotStyle::Bars,
            capacity: Self::DEFAULT_CAPACITY,
            width: Self::DEFAULT_SIZE[0],
            height: Self::DEFAULT_SIZE[1],
            color: None,
            show_grid: false,
            show_axes: false,
            interactive: false,
            margin: true,
            y_min: None,
            y_max: None,
        }
    }
}

/// Append `val` to the scope history `state`, dropping oldest entries so the result
/// holds at most `cap` items. Registered on the VM as `plot-push` and called from the
/// generated [`PlotMode::Scope`] expression.
///
/// A numeric `val` is appended; a list *or vector* `val` extends the history with its
/// numeric elements; a container *of containers* (e.g. `~scopeout`'s per-channel
/// rings) extends one history per inner container - the state becomes a vector of
/// per-channel vectors (the shape [`split_channels`] renders as stacked sub-plots),
/// each capped at `cap` independently. Anything else is ignored. The history follows
/// the incoming shape: a prior state of the other shape (flat vs per-channel) is
/// discarded. `cap` is passed as an argument (not captured) so a single shared
/// `plot-push` serves every plot node with its own, always-current capacity.
///
/// Histories are kept as persistent vectors ([`SteelVal::VectorV`], an `im_rc::Vector`)
/// rather than lists: `push_back`/`pop_front` are O(1) amortised (vs a steel list's O(n)
/// `push_back`), so a whole incoming `~scopeout` window can be appended sample-by-sample
/// cheaply - no full rebuild, and the single-sample scope push is O(log n) instead of O(n).
fn plot_push(state: SteelVal, val: SteelVal, cap: SteelVal) -> SteelVal {
    let cap = match cap {
        SteelVal::IntV(n) if n > 0 => n as usize,
        _ => 0,
    };

    // Per-channel data: each inner container extends its own channel's history,
    // reusing the prior history where the state is already per-channel.
    if let Some(chans) = per_channel_elems(&val) {
        let old: Vec<SteelVal> = match &state {
            SteelVal::VectorV(v) if v.iter().any(is_container) => v.iter().cloned().collect(),
            SteelVal::ListV(l) if l.iter().any(is_container) => l.iter().cloned().collect(),
            _ => Vec::new(),
        };
        let channels: Vector<SteelVal> = chans
            .iter()
            .enumerate()
            .map(|(c, ch)| {
                let history = push_capped(history_of(old.get(c)), ch, cap);
                SteelVal::VectorV(Gc::new(history).into())
            })
            .collect();
        return SteelVal::VectorV(Gc::new(channels).into());
    }

    // Flat numeric scope: append to the one shared history.
    let history = push_capped(history_of(Some(&state)), &val, cap);
    SteelVal::VectorV(Gc::new(history).into())
}

/// Whether `v` is a numeric [`SteelVal`].
fn is_num(v: &SteelVal) -> bool {
    matches!(v, SteelVal::NumV(_) | SteelVal::IntV(_))
}

/// The top-level elements of a container-of-containers `val` (per-channel data,
/// e.g. `~scopeout`'s rings); `None` for a flat container, a number, or anything
/// else.
fn per_channel_elems(val: &SteelVal) -> Option<Vec<SteelVal>> {
    let elems: Vec<SteelVal> = match val {
        SteelVal::ListV(list) => list.iter().cloned().collect(),
        SteelVal::VectorV(vec) => vec.iter().cloned().collect(),
        _ => return None,
    };
    elems.iter().any(is_container).then_some(elems)
}

/// One channel's existing scope history: the prior vector (a structural, O(1)
/// clone) or a prior *flat numeric* list's numbers (e.g. after a signal->scope
/// switch); a per-channel vector (containers inside) or absent value is empty -
/// the history follows the incoming shape.
fn history_of(state: Option<&SteelVal>) -> Vector<SteelVal> {
    match state {
        Some(SteelVal::VectorV(v)) if !v.iter().any(is_container) => (**v).clone(),
        Some(SteelVal::ListV(list)) => list.iter().filter(|v| is_num(v)).cloned().collect(),
        _ => Vector::new(),
    }
}

/// Append the incoming value's numeric samples to `history` and cap it: a list or
/// vector contributes its numeric elements; a lone number contributes itself;
/// anything else nothing.
fn push_capped(mut history: Vector<SteelVal>, val: &SteelVal, cap: usize) -> Vector<SteelVal> {
    match val {
        SteelVal::ListV(items) => {
            for v in items.iter().filter(|v| is_num(v)) {
                history.push_back(v.clone());
            }
        }
        SteelVal::VectorV(items) => {
            for v in items.iter().filter(|v| is_num(v)) {
                history.push_back(v.clone());
            }
        }
        num @ (SteelVal::NumV(_) | SteelVal::IntV(_)) => history.push_back(num.clone()),
        _ => {}
    }
    while history.len() > cap {
        history.pop_front();
    }
    history
}

/// Read the node's stored series as per-channel `f64`s (see [`split_channels`]): a
/// list or vector yields its numeric elements; a lone number a single sample; anything
/// else is empty.
fn series(ctx: &NodeCtx) -> Vec<Vec<f64>> {
    match ctx.extract_value() {
        Ok(Some(val)) => split_channels(&val),
        _ => Vec::new(),
    }
}

impl gantz_core::Node for Plot {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        // The node forwards its input unchanged (pass-through) while capturing
        // the series to plot into `state`.
        let expr = match ctx.inputs().get(0) {
            Some(Some(val)) => match self.mode {
                // Append the incoming number (or list elements) to the history;
                // `plot-push` ignores anything non-numeric.
                PlotMode::Scope => format!(
                    "(begin (set! state (plot-push state {val} {cap})) {val})",
                    cap = self.capacity,
                ),
                // Store the incoming value directly.
                PlotMode::Signal => format!("(begin (set! state {val}) {val})"),
            },
            // No input connected: nothing to capture or forward; yield the
            // stored series (mirrors `inspect`'s unconnected behaviour).
            _ => "(begin state)".to_string(),
        };
        node::parse_expr(&expr)
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        node::state::init_value_if_absent(ctx.vm(), path, || {
            SteelVal::VectorV(std::iter::empty::<SteelVal>().collect())
        })
        .unwrap();
        // Register the shared `plot-push` helper, but only if absent. Steel's
        // `register_fn` allocates a *new* global slot and shadows the previous
        // binding rather than overwriting it, so re-registering on every
        // recompile (the engine persists across them) would leak the old
        // closure - a slow memory leak as the plot is edited. One binding is
        // shared by every plot node.
        if ctx.vm().extract_value("plot-push").is_err() {
            ctx.vm().register_fn("plot-push", plot_push);
        }
    }
}

impl Plot {
    /// The plot's fragment, bound to its own state, with attrs baked from
    /// the weight. `size` is the resolved body size (the resize container's
    /// inner size); `None` fills the available space (the detached view).
    fn fragment(&self, id: node::Id, size: Option<egui::Vec2>) -> gantz_ui::Element {
        gantz_ui::Element::Plot(gantz_ui::Plot {
            bind: Some(gantz_ui::BindPath(vec![id])),
            mode: Some(match self.mode {
                PlotMode::Scope => gantz_ui::PlotMode::Scope,
                PlotMode::Signal => gantz_ui::PlotMode::Signal,
            }),
            style: Some(match self.style {
                PlotStyle::Bars => gantz_ui::PlotStyle::Bars,
                PlotStyle::Line => gantz_ui::PlotStyle::Line,
            }),
            color: self.color.map(gantz_ui::Rgba),
            grid: self.show_grid,
            axes: self.show_axes,
            interactive: self.interactive,
            y_min: self.y_min.map(F32::get),
            y_max: self.y_max.map(F32::get),
            w: size.map(|s| s.x),
            h: size.map(|s| s.y),
            key: None,
        })
    }
}

impl NodeUi for Plot {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "plot".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Plot incoming values as a scrolling scope or a signal/array")
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // Set when a settled resize commits a new (CA-affecting) body size.
        let mut changed = false;

        let style = uictx.style();
        let interaction = uictx.interaction();

        // A minimal extreme-bg frame. The `margin` toggle controls whether the
        // data is inset by the frame's regular margin (with rounded corners) or
        // fills it edge-to-edge (with square corners, so nothing is clipped).
        let mut frame = egui_graph::node::default_frame(style, interaction);
        frame.fill = style.visuals.extreme_bg_color;
        if !self.margin {
            frame.inner_margin = egui::Margin::ZERO;
            frame.corner_radius = egui::CornerRadius::ZERO;
        }

        let node_egui_id = uictx.egui_id();
        let resize_id = node_egui_id.with("resize");
        let root_id = node_egui_id.with("gui");
        let min_size = egui::Vec2::splat(style.interaction.interact_radius * 2.0);
        let default_size = egui::vec2(self.width as f32, self.height as f32);

        let (&id, prefix) = ctx.path().split_last().expect("a node path is never empty");

        let size_sync_id = node_egui_id.with("size_sync");
        let framed = uictx.framed_with(frame, |ui, _sockets| {
            let size_sync::Decisions {
                resizing,
                push_external,
                drag_released,
            } = size_sync::begin(ui, size_sync_id, resize_id, [self.width, self.height]);

            let resize = egui::containers::Resize::default()
                .id(resize_id)
                .with_stroke(false);
            let resize = if push_external {
                // One-frame push of the committed size into the displayed
                // resize state (see `node::size_sync`): overrides persisted
                // state and cancels any in-flight drag - external wins.
                ui.ctx().request_repaint();
                let w = (self.width as f32).max(min_size.x);
                let h = (self.height as f32).max(min_size.y);
                resize.fixed_size(egui::vec2(w, h))
            } else {
                // Both axes are user-resizable while the node is selected.
                let resizable = egui::Vec2b::new(interaction.selected, interaction.selected);
                resize
                    .resizable(resizable)
                    .default_size(default_size)
                    .min_size(min_size)
            };
            let inner = resize.show(ui, |ui| {
                let avail = ui.available_size();

                // `size` is part of the content address, so it is written
                // only on a settled corner-drag release - never mid-drag
                // (a commit per drag frame), and never merely because the
                // rendered size differs (which would clobber external
                // changes from undo/collab sync and mint spurious commits).
                let fitted = fitted_size(avail.x.max(min_size.x), avail.y.max(min_size.y));
                if drag_released && [self.width, self.height] != fitted {
                    [self.width, self.height] = fitted;
                    changed = true;
                }

                let tree = self.fragment(id, Some(avail));
                let r = UiTree::new(root_id)
                    .instance_prefix(prefix)
                    .show(&tree, &mut ctx, ui);
                r.inner.unwrap_or_else(|| ui.response())
            });

            size_sync::store(
                ui,
                size_sync_id,
                [self.width, self.height],
                push_external,
                resizing,
            );

            inner
        });

        let mut resp = NodeUiResponse::new(framed);
        resp.set_changed(changed);
        resp
    }

    fn view_no_margin(&self) -> bool {
        // The plot fills its pane edge-to-edge, with no surrounding margin.
        true
    }

    fn view_ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
        // The detached view fills the pane (the fragment omits w/h). Unlike
        // the in-graph body it has no resize handle and never writes back
        // the node's CA-affecting `width`/`height` (so `changed` stays
        // false). The root id is derived from `ui` (scoped per pane by the
        // caller), keeping it distinct from the in-graph plot's id.
        let (&id, prefix) = ctx.path().split_last().expect("a node path is never empty");
        let tree = self.fragment(id, None);
        let r = UiTree::new(ui.id().with("gui"))
            .instance_prefix(prefix)
            .show(&tree, &mut ctx, ui);
        let mut out = NodeViewResponse::default();
        out.inner = r.inner;
        out.payloads = r.payloads;
        out
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        let mut changed = false;

        // A summarised replacement for the (suppressed) default state row - the
        // raw history would be a huge list.
        let chans = series(ctx);
        let total: usize = chans.iter().map(Vec::len).sum();
        let summary = if chans.len() > 1 {
            format!("{total} samples · {} channels", chans.len())
        } else {
            format!("{total} samples")
        };
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("state");
            });
            row.col(|ui| {
                ui.label(summary);
            });
        });

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("mode");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    changed |= radio_option(
                        ui,
                        &mut self.mode,
                        PlotMode::Scope,
                        "scope",
                        "accumulate a scrolling history",
                    );
                    changed |= radio_option(
                        ui,
                        &mut self.mode,
                        PlotMode::Signal,
                        "signal",
                        "plot the incoming value directly",
                    );
                });
            });
        });

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("style");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    changed |= radio_option(
                        ui,
                        &mut self.style,
                        PlotStyle::Bars,
                        "bars",
                        "draw as contiguous bars",
                    );
                    changed |= radio_option(
                        ui,
                        &mut self.style,
                        PlotStyle::Line,
                        "line",
                        "draw as a connected line",
                    );
                });
            });
        });

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("capacity");
            });
            row.col(|ui| {
                let mut c = self.capacity as i32;
                if ui
                    .add(egui::DragValue::new(&mut c).range(1..=4096).speed(1.0))
                    .on_hover_text("max samples retained in scope mode")
                    .changed()
                {
                    self.capacity = c.clamp(1, 4096) as u32;
                    changed = true;
                }
            });
        });

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("margin");
            });
            row.col(|ui| {
                if ui
                    .checkbox(&mut self.margin, "")
                    .on_hover_text(
                        "inset the data within the node frame's margin (rounded corners)",
                    )
                    .changed()
                {
                    changed = true;
                }
            });
        });

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("colour");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    let mut col = resolve_color(self.color, ui);
                    if ui
                        .color_edit_button_srgba(&mut col)
                        .on_hover_text("the line/bar colour")
                        .changed()
                    {
                        self.color = Some([col.r(), col.g(), col.b(), col.a()]);
                        changed = true;
                    }
                    if self.color.is_some()
                        && ui
                            .button("theme")
                            .on_hover_text("follow the theme's strong text colour")
                            .clicked()
                    {
                        self.color = None;
                        changed = true;
                    }
                });
            });
        });

        // Min and max are two columns of one grid row (hover text says which is
        // which), so the max controls stay put as the min dialer's value width
        // changes (the dialers have a fixed width).
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("range");
            });
            row.col(|ui| {
                egui::Grid::new("plot_range").num_columns(2).show(ui, |ui| {
                    let mut y_min = self.y_min.map(F32::get);
                    if node_inspector::bound_col(ui, "minimum", &mut y_min) {
                        self.y_min = y_min.map(F32);
                        changed = true;
                    }
                    let mut y_max = self.y_max.map(F32::get);
                    if node_inspector::bound_col(ui, "maximum", &mut y_max) {
                        self.y_max = y_max.map(F32);
                        changed = true;
                    }
                    ui.end_row();
                });
            });
        });

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("display");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    changed |= ui
                        .checkbox(&mut self.show_grid, "grid")
                        .on_hover_text("draw the background grid")
                        .changed();
                    changed |= ui
                        .checkbox(&mut self.show_axes, "axes")
                        .on_hover_text("draw the axes")
                        .changed();
                    changed |= ui
                        .checkbox(&mut self.interactive, "interactive")
                        .on_hover_text("show a crosshair and value readout on hover")
                        .changed();
                });
            });
        });

        let mut resp = InspectorRowsResponse::default();
        resp.set_changed(changed);
        resp
    }

    fn context_menu(&mut self, ctx: &mut NodeCtx, ui: &mut egui::Ui) -> ContextMenuResponse {
        if ui
            .button("clear history")
            .on_hover_text("empty the plotted series")
            .clicked()
        {
            // VM runtime state, not content-addressed: do not mark changed.
            ctx.update_value(SteelVal::VectorV(std::iter::empty::<SteelVal>().collect()))
                .ok();
            ui.close();
        }
        ContextMenuResponse::default()
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => SocketDoc::ty("number or list").with_description(
                "scope: a number (or list) appended to the history; signal: the value to plot",
            ),
            SocketKind::Output => {
                SocketDoc::ty("any").with_description("the input value, unchanged")
            }
        })
    }

    fn show_state(&self) -> bool {
        // The raw history is a long list; the inspector summarises it instead.
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_tree::plot::steel_num;
    use gantz_core::node::{Node, WithPushEval};
    use gantz_core::{
        Edge, ROOT_STATE,
        compile::{entry_fn_name, entrypoint, push_pull_entrypoints},
    };
    use steel::steel_vm::engine::Engine;

    // A node lookup is unnecessary for these self-contained graphs.
    fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
        None
    }

    // Compile `g`, init a base VM with node state, and load the module.
    fn vm_for(g: &petgraph::graph::DiGraph<Box<dyn Node>, Edge>) -> Engine {
        let eps = push_pull_entrypoints(&no_lookup, g);
        let module = gantz_core::compile::module(&no_lookup, g, &eps, &Default::default()).unwrap();
        let mut vm = Engine::new_base();
        vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
        gantz_core::graph::register(&no_lookup, g, &[], &mut vm);
        for f in module {
            vm.run(format!("{f}")).unwrap();
        }
        vm
    }

    // Fire the push entrypoint of node `ix` `n` times.
    fn fire(
        vm: &mut Engine,
        g: &petgraph::graph::DiGraph<Box<dyn Node>, Edge>,
        ix: usize,
        n: usize,
    ) {
        let ctx = node::MetaCtx::new(&no_lookup);
        let outs = g[petgraph::graph::NodeIndex::new(ix)].n_outputs(ctx) as u8;
        let ep = entrypoint::push(vec![ix], outs);
        let fn_name = entry_fn_name(&ep.id());
        for _ in 0..n {
            vm.call_function_by_name_with_args(&fn_name, vec![])
                .unwrap();
        }
    }

    // Read a node's stored numeric samples, whether stored as a list or a vector.
    fn samples_of(vm: &Engine, ix: usize) -> Vec<f64> {
        match node::state::extract_value(vm, &[ix]).unwrap().unwrap() {
            SteelVal::ListV(list) => list.iter().filter_map(steel_num).collect(),
            SteelVal::VectorV(vec) => vec.iter().filter_map(steel_num).collect(),
            other => panic!("expected list/vector state, got {other:?}"),
        }
    }

    // Build `src -> plot`, returning the graph and the two node indices.
    fn graph_with(
        src: Box<dyn Node>,
        plot: Plot,
    ) -> (petgraph::graph::DiGraph<Box<dyn Node>, Edge>, usize, usize) {
        let mut g = petgraph::graph::DiGraph::new();
        let s = g.add_node(src);
        let p = g.add_node(Box::new(plot) as Box<dyn Node>);
        g.add_edge(s, p, Edge::from((0, 0)));
        (g, s.index(), p.index())
    }

    // Scope mode appends each pushed number and bounds the history to `capacity`.
    #[test]
    fn scope_accumulates_bounded_history() {
        let src = gantz_core::node::expr("5").unwrap().with_push_eval();
        let plot = Plot {
            mode: PlotMode::Scope,
            capacity: 3,
            ..Default::default()
        };
        let (g, s, p) = graph_with(Box::new(src) as Box<dyn Node>, plot);
        let mut vm = vm_for(&g);
        fire(&mut vm, &g, s, 5);
        assert_eq!(samples_of(&vm, p), vec![5.0, 5.0, 5.0]);
    }

    // Scope mode extends the history with a pushed list's elements.
    #[test]
    fn scope_extends_with_list() {
        let src = gantz_core::node::expr("(list 1 2 3)")
            .unwrap()
            .with_push_eval();
        let plot = Plot {
            mode: PlotMode::Scope,
            capacity: 10,
            ..Default::default()
        };
        let (g, s, p) = graph_with(Box::new(src) as Box<dyn Node>, plot);
        let mut vm = vm_for(&g);
        fire(&mut vm, &g, s, 2);
        assert_eq!(samples_of(&vm, p), vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]);
    }

    // A pushed list at least as long as the capacity keeps only its last `cap`
    // samples - the bulk fast path drops the prior history without reading it.
    #[test]
    fn scope_list_over_capacity_keeps_tail() {
        let src = gantz_core::node::expr("(list 1 2 3 4 5)")
            .unwrap()
            .with_push_eval();
        let plot = Plot {
            mode: PlotMode::Scope,
            capacity: 3,
            ..Default::default()
        };
        let (g, s, p) = graph_with(Box::new(src) as Box<dyn Node>, plot);
        let mut vm = vm_for(&g);
        fire(&mut vm, &g, s, 1);
        assert_eq!(samples_of(&vm, p), vec![3.0, 4.0, 5.0]);
        // A second identical window still yields just its last 3 (history dropped).
        fire(&mut vm, &g, s, 1);
        assert_eq!(samples_of(&vm, p), vec![3.0, 4.0, 5.0]);
    }

    // A pushed list *of channels* (e.g. `~scopeout`'s per-channel rings) accumulates
    // one capped history per channel - the stacked-sub-plot state shape.
    #[test]
    fn scope_accumulates_per_channel_histories() {
        let src = gantz_core::node::expr("(list (list 1 2) (list -1 -2))")
            .unwrap()
            .with_push_eval();
        let plot = Plot {
            mode: PlotMode::Scope,
            capacity: 3,
            ..Default::default()
        };
        let (g, s, p) = graph_with(Box::new(src) as Box<dyn Node>, plot);
        let mut vm = vm_for(&g);
        // Two windows of 2 samples with capacity 3: each channel keeps its last 3.
        fire(&mut vm, &g, s, 2);
        let state = node::state::extract_value(&vm, &[p]).unwrap().unwrap();
        assert_eq!(
            split_channels(&state),
            vec![vec![2.0, 1.0, 2.0], vec![-2.0, -1.0, -2.0]],
        );
    }

    // The history follows the incoming shape: a flat history is discarded when
    // per-channel data arrives (and vice versa), rather than mixing shapes.
    #[test]
    fn scope_shape_switch_discards_prior_history() {
        let num = |n: f64| SteelVal::NumV(n);
        let list = |vals: Vec<SteelVal>| SteelVal::ListV(vals.into_iter().collect());
        let cap = SteelVal::IntV(8);

        // Flat history + per-channel value: the flat samples are discarded.
        let flat = plot_push(SteelVal::Void, num(1.0), cap.clone());
        let chans = plot_push(flat, list(vec![list(vec![num(2.0)])]), cap.clone());
        assert_eq!(split_channels(&chans), vec![vec![2.0]]);

        // Per-channel history + flat value: the channel histories are discarded.
        let flat_again = plot_push(chans, num(3.0), cap);
        assert_eq!(split_channels(&flat_again), vec![vec![3.0]]);
    }

    // When a pushed list overflows the *remaining* capacity, the oldest history is
    // trimmed so history-tail + new totals `cap`.
    #[test]
    fn scope_list_trims_oldest_to_cap() {
        let src = gantz_core::node::expr("(list 1 2 3)")
            .unwrap()
            .with_push_eval();
        let plot = Plot {
            mode: PlotMode::Scope,
            capacity: 4,
            ..Default::default()
        };
        let (g, s, p) = graph_with(Box::new(src) as Box<dyn Node>, plot);
        let mut vm = vm_for(&g);
        // [1,2,3], then keep the last 4 of [1,2,3] ++ [1,2,3] = [3,1,2,3].
        fire(&mut vm, &g, s, 2);
        assert_eq!(samples_of(&vm, p), vec![3.0, 1.0, 2.0, 3.0]);
    }

    // `plot_push` accepts a vector input, accumulating its numeric elements into the
    // vector-backed scope history and capping at `cap`. (A vector-emitting Steel expr
    // isn't available under `new_base`, so exercise the fn directly.)
    #[test]
    fn plot_push_accepts_vector() {
        let num = |n: f64| SteelVal::NumV(n);
        let vector = |xs: Vec<SteelVal>| SteelVal::VectorV(xs.into_iter().collect());
        let empty = SteelVal::VectorV(std::iter::empty::<SteelVal>().collect());

        let s1 = plot_push(
            empty,
            vector(vec![num(1.0), num(2.0), num(3.0)]),
            SteelVal::IntV(4),
        );
        let s2 = plot_push(s1, vector(vec![num(4.0), num(5.0)]), SteelVal::IntV(4));

        // The history is a VectorV of the last 4 samples, in order.
        let got: Vec<f64> = match s2 {
            SteelVal::VectorV(v) => v.iter().filter_map(steel_num).collect(),
            other => panic!("expected vector state, got {other:?}"),
        };
        assert_eq!(got, vec![2.0, 3.0, 4.0, 5.0]);
    }

    // Signal mode stores the incoming list verbatim, preserving order.
    #[test]
    fn signal_stores_list() {
        let src = gantz_core::node::expr("(list 1 2 3)")
            .unwrap()
            .with_push_eval();
        let plot = Plot {
            mode: PlotMode::Signal,
            ..Default::default()
        };
        let (g, s, p) = graph_with(Box::new(src) as Box<dyn Node>, plot);
        let mut vm = vm_for(&g);
        fire(&mut vm, &g, s, 1);
        assert_eq!(samples_of(&vm, p), vec![1.0, 2.0, 3.0]);
    }

    // Signal mode also accepts a single number (drawn as one bar).
    #[test]
    fn signal_stores_scalar() {
        let src = gantz_core::node::expr("7").unwrap().with_push_eval();
        let plot = Plot {
            mode: PlotMode::Signal,
            ..Default::default()
        };
        let (g, s, p) = graph_with(Box::new(src) as Box<dyn Node>, plot);
        let mut vm = vm_for(&g);
        fire(&mut vm, &g, s, 1);
        // Stored as a lone number; `series` reads it as a single sample.
        let state = node::state::extract_value(&vm, &[p]).unwrap().unwrap();
        assert!(matches!(state, SteelVal::IntV(7)));
    }

    // The fragment bakes every render-relevant weight field, the bind id,
    // and the resolved body size (absent for the fill-the-pane view).
    #[test]
    fn fragment_bakes_weight_attrs_and_bind() {
        let plot = Plot {
            mode: PlotMode::Signal,
            style: PlotStyle::Line,
            color: Some([1, 2, 3, 4]),
            show_grid: true,
            show_axes: true,
            interactive: true,
            y_min: Some(F32(-1.0)),
            y_max: Some(F32(1.0)),
            ..Default::default()
        };
        let expected = gantz_ui::Element::Plot(gantz_ui::Plot {
            bind: Some(gantz_ui::BindPath(vec![4])),
            mode: Some(gantz_ui::PlotMode::Signal),
            style: Some(gantz_ui::PlotStyle::Line),
            color: Some(gantz_ui::Rgba([1, 2, 3, 4])),
            grid: true,
            axes: true,
            interactive: true,
            y_min: Some(-1.0),
            y_max: Some(1.0),
            w: Some(120.0),
            h: Some(80.0),
            key: None,
        });
        assert_eq!(plot.fragment(4, Some(egui::vec2(120.0, 80.0))), expected);

        // The view fragment omits w/h to fill the pane.
        let gantz_ui::Element::Plot(p) = plot.fragment(4, None) else {
            panic!("plot fragment is a plot element");
        };
        assert_eq!((p.w, p.h), (None, None));
    }

    // Registering the graph again on the same engine (as a recompile does) must
    // keep `plot-push` working - the registration guard must not skip the first
    // registration, and must not break on the second. (The guard also prevents a
    // leaked global binding per recompile.)
    #[test]
    fn re_registration_keeps_plot_push_working() {
        let src = gantz_core::node::expr("5").unwrap().with_push_eval();
        let plot = Plot {
            mode: PlotMode::Scope,
            capacity: 3,
            ..Default::default()
        };
        let (g, s, p) = graph_with(Box::new(src) as Box<dyn Node>, plot);
        let mut vm = vm_for(&g);
        // A second registration pass over the same engine.
        gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
        fire(&mut vm, &g, s, 5);
        assert_eq!(samples_of(&vm, p), vec![5.0, 5.0, 5.0]);
    }
}
