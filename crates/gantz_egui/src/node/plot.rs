//! A Plot node for visualising numeric values flowing through the graph.
//!
//! The node body is intentionally minimal - just a plot - while its appearance
//! and behaviour are configured through the node inspector and context menu.
//!
//! Two modes are supported:
//! - [`PlotMode::Scope`]: accumulate a bounded, scrolling history and plot it
//!   like an oscilloscope. Each pushed number is appended; a pushed list extends
//!   the history with its elements.
//! - [`PlotMode::Signal`]: plot the incoming value directly (a list as a series,
//!   a single number as one bar), replacing it on each evaluation.
//!
//! In both modes the node is a pass-through: its output forwards the input
//! value unchanged (like [`super::Inspect`]), so a value can be observed without
//! breaking the chain it flows through.

use crate::widget::node_inspector;
use crate::widget::node_inspector::radio_option;
use crate::{
    ContextMenuResponse, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, Registry,
    SocketDoc, SocketKind,
};
use gantz_ca::CaHash;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use serde::{Deserialize, Serialize};
use steel::SteelVal;
use steel::steel_vm::register_fn::RegisterFn;

/// An `f32` that participates in content addressing and `Hash` via its bit
/// pattern, letting float-valued config keep [`Plot`]'s derives (the `CaHash`
/// derive needs every field to be `CaHash`, and the app's `dyn Node` needs
/// `Hash` - neither is implemented for `f32`).
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

impl CaHash for F32 {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        CaHash::hash(&self.0.to_bits(), hasher);
    }
}

/// How the plot interprets and accumulates its input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
pub enum PlotMode {
    /// Accumulate a bounded scrolling history (numbers appended, lists extend).
    Scope,
    /// Plot the incoming value directly, replacing the prior.
    Signal,
}

/// How the series is drawn.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
pub enum PlotStyle {
    /// Contiguous bars (the default).
    Bars,
    /// A connected line.
    Line,
}

/// A node that plots the numeric values it receives.
///
/// Every field feeds the content address (no `#[cahash(skip)]`), so each
/// inspector edit is a real, persisted, undoable change rather than transient
/// view state.
#[derive(Clone, Debug, Hash, Deserialize, Serialize, CaHash)]
#[cahash("gantz.plot")]
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

/// Append `val` to the scope history `state`, dropping oldest entries so the
/// result holds at most `cap` items. Registered on the VM as `plot-push` and
/// called from the generated [`PlotMode::Scope`] expression.
///
/// A numeric `val` is appended; a list `val` extends the history with its
/// numeric elements; anything else is ignored. `cap` is passed as an argument
/// (not captured) so a single shared `plot-push` serves every plot node with its
/// own, always-current capacity.
fn plot_push(state: SteelVal, val: SteelVal, cap: SteelVal) -> SteelVal {
    let cap = match cap {
        SteelVal::IntV(n) if n > 0 => n as usize,
        _ => 0,
    };
    // Reuse the existing list, or start fresh if state isn't a list yet.
    let mut list = match state {
        SteelVal::ListV(list) => list,
        _ => Default::default(),
    };
    match val {
        SteelVal::ListV(items) => {
            for item in items.iter() {
                if matches!(item, SteelVal::NumV(_) | SteelVal::IntV(_)) {
                    list.push_back(item.clone());
                }
            }
        }
        num @ (SteelVal::NumV(_) | SteelVal::IntV(_)) => list.push_back(num),
        _ => {}
    }
    while list.len() > cap {
        list.pop_front();
    }
    SteelVal::ListV(list)
}

/// Read the node's stored series as `f64`s. A list yields its numeric elements;
/// a lone number yields a single sample; anything else is empty.
fn series(ctx: &NodeCtx) -> Vec<f64> {
    match ctx.extract_value() {
        Ok(Some(SteelVal::ListV(list))) => list.iter().filter_map(steel_num).collect(),
        Ok(Some(ref val)) => steel_num(val).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Convert a numeric [`SteelVal`] to `f64`.
fn steel_num(val: &SteelVal) -> Option<f64> {
    match val {
        SteelVal::NumV(f) => Some(*f),
        SteelVal::IntV(i) => Some(*i as f64),
        _ => None,
    }
}

/// Resolve the configured colour, falling back to the theme's strong text
/// colour when unset.
fn resolve_color(color: Option<[u8; 4]>, ui: &egui::Ui) -> egui::Color32 {
    match color {
        Some([r, g, b, a]) => egui::Color32::from_rgba_unmultiplied(r, g, b, a),
        None => ui.visuals().strong_text_color(),
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
        node::state::init_value_if_absent(ctx.vm(), path, || SteelVal::ListV(Default::default()))
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

impl NodeUi for Plot {
    fn name(&self, _: &dyn Registry) -> &str {
        "plot"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Plot incoming values as a scrolling scope or a signal/array")
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
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
        let plot_id = node_egui_id.with("plot");
        let min_size = egui::Vec2::splat(style.interaction.interact_radius * 2.0);
        let default_size = egui::vec2(self.width as f32, self.height as f32);

        // Read the series once, up-front (only borrows `ctx`).
        let ys = series(&ctx);

        let framed = uictx.framed_with(frame, |ui, _sockets| {
            // `Resize` registers its corner under this salt; reading last frame's
            // response tells us whether it is being actively dragged.
            let corner_id = resize_id.with("__resize_corner");
            let resizing = ui
                .ctx()
                .read_response(corner_id)
                .is_some_and(|r| r.dragged());
            if resizing {
                ui.ctx().request_repaint();
            }

            // Both axes are user-resizable while the node is selected.
            let resizable = egui::Vec2b::new(interaction.selected, interaction.selected);
            egui::containers::Resize::default()
                .id(resize_id)
                .resizable(resizable)
                .default_size(default_size)
                .min_size(min_size)
                .with_stroke(false)
                .show(ui, |ui| {
                    let avail = ui.available_size();

                    // `size` is part of the content address, so only commit it
                    // once *settled* - never mid-drag, which would churn a commit
                    // every frame.
                    let new_w = avail.x.max(min_size.x).round() as u16;
                    let new_h = avail.y.max(min_size.y).round() as u16;
                    if !resizing && (self.width != new_w || self.height != new_h) {
                        self.width = new_w;
                        self.height = new_h;
                        changed = true;
                    }

                    let color = resolve_color(self.color, ui);
                    let plot_style = self.style;
                    let interactive = self.interactive;
                    let bounds = value_bounds(&ys, plot_style, self.y_min, self.y_max);

                    let mut plot = egui_plot::Plot::new(plot_id)
                        .width(avail.x)
                        .height(avail.y)
                        .show_background(false)
                        .show_axes(egui::Vec2b::new(self.show_axes, self.show_axes))
                        .show_grid(egui::Vec2b::new(self.show_grid, self.show_grid))
                        // Pan/zoom are always off. `Sense::hover` lets the node
                        // frame beneath capture drags and right-clicks, so the
                        // node moves and its context menu opens as usual.
                        .allow_drag(false)
                        .allow_zoom(false)
                        .allow_scroll(false)
                        .allow_boxed_zoom(false)
                        .sense(egui::Sense::hover());
                    if !interactive {
                        // Purely visual: hide the crosshair (the value readout is
                        // also suppressed via `allow_hover(false)` below).
                        plot = plot.cursor_color(egui::Color32::TRANSPARENT);
                    }

                    let plot_resp = plot
                        .show(ui, |plot_ui| {
                            match plot_style {
                                PlotStyle::Bars => {
                                    let bars = ys
                                        .iter()
                                        .enumerate()
                                        .map(|(i, &y)| {
                                            egui_plot::Bar::new(i as f64, y)
                                                .width(1.0)
                                                .fill(color)
                                                .stroke(egui::Stroke::NONE)
                                        })
                                        .collect();
                                    plot_ui.bar_chart(
                                        egui_plot::BarChart::new("", bars).allow_hover(interactive),
                                    );
                                }
                                PlotStyle::Line => {
                                    let points = egui_plot::PlotPoints::from_ys_f64(&ys);
                                    plot_ui.line(
                                        egui_plot::Line::new("", points)
                                            .color(color)
                                            .allow_hover(interactive),
                                    );
                                }
                            }
                            // Drive the view deterministically from the data +
                            // config (the plot never pans), so live updates and
                            // min/max apply.
                            let ([xlo, ylo], [xhi, yhi]) = bounds;
                            plot_ui.set_plot_bounds_x(xlo..=xhi);
                            plot_ui.set_plot_bounds_y(ylo..=yhi);
                        })
                        .response;

                    // egui_plot sets a crosshair *mouse cursor* on hover; when not
                    // interactive, restore the default arrow so the plot reads as a
                    // static node. (The resize corner sets its own cursor after
                    // this, so it is unaffected.)
                    if !interactive && plot_resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
                    }
                    plot_resp
                })
        });

        let mut resp = NodeUiResponse::new(framed);
        resp.set_changed(changed);
        resp
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
        let n_samples = series(ctx).len();
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("state");
            });
            row.col(|ui| {
                ui.label(format!("{n_samples} samples"));
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
            ctx.update_value(SteelVal::ListV(Default::default())).ok();
            ui.close();
        }
        ContextMenuResponse::default()
    }

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
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

/// Compute `([x_min, y_min], [x_max, y_max])` for the view from the data and
/// optional fixed value bounds. Bars include the baseline `0` and span integer
/// x; lines span sample indices. The plot itself adds no margin.
fn value_bounds(
    ys: &[f64],
    style: PlotStyle,
    y_min: Option<F32>,
    y_max: Option<F32>,
) -> ([f64; 2], [f64; 2]) {
    let n = ys.len() as f64;
    let (xlo, xhi) = match style {
        PlotStyle::Bars => (-0.5, (n - 0.5).max(0.5)),
        PlotStyle::Line => (0.0, (n - 1.0).max(1.0)),
    };

    let (dmin, dmax) = ys
        .iter()
        .copied()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
    let (mut ylo, mut yhi) = if dmin <= dmax {
        match style {
            // Bars draw from the baseline, so keep `0` in view.
            PlotStyle::Bars => (dmin.min(0.0), dmax.max(0.0)),
            PlotStyle::Line => (dmin, dmax),
        }
    } else {
        (0.0, 1.0)
    };
    if (yhi - ylo).abs() < 1e-9 {
        ylo -= 1.0;
        yhi += 1.0;
    }

    // Fixed overrides are exact.
    if let Some(v) = y_min {
        ylo = v.get() as f64;
    }
    if let Some(v) = y_max {
        yhi = v.get() as f64;
    }

    ([xlo, ylo], [xhi, yhi])
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn list_of(vm: &Engine, ix: usize) -> Vec<f64> {
        match node::state::extract_value(vm, &[ix]).unwrap().unwrap() {
            SteelVal::ListV(list) => list.iter().filter_map(steel_num).collect(),
            other => panic!("expected list state, got {other:?}"),
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
        assert_eq!(list_of(&vm, p), vec![5.0, 5.0, 5.0]);
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
        assert_eq!(list_of(&vm, p), vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]);
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
        assert_eq!(list_of(&vm, p), vec![1.0, 2.0, 3.0]);
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
        assert_eq!(list_of(&vm, p), vec![5.0, 5.0, 5.0]);
    }
}
