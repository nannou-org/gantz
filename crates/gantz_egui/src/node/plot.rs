//! A Plot node for visualising numeric values flowing through the graph.
//!
//! The node body is intentionally minimal - just a plot - while its appearance
//! and behaviour are configured through the node inspector and context menu.
//!
//! Two modes are supported:
//! - [`PlotMode::Scope`]: the input is a single number; the node accumulates a
//!   bounded, scrolling history and plots it like an oscilloscope.
//! - [`PlotMode::Signal`]: the input is a list of numbers; the node plots the
//!   whole list directly, replacing it on each evaluation.
//!
//! In both modes the node is a pass-through: its output forwards the input
//! value unchanged (like [`super::Inspect`]), so a value can be observed without
//! breaking the chain it flows through.

use crate::widget::node_inspector;
use crate::{NodeCtx, NodeUi, Registry, SocketDoc, SocketKind};
use gantz_ca::CaHash;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use serde::{Deserialize, Serialize};
use steel::SteelVal;
use steel::steel_vm::register_fn::RegisterFn;

/// How the plot interprets and accumulates its input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
pub enum PlotMode {
    /// The input is a single number; keep a bounded scrolling history.
    Scope,
    /// The input is a list of numbers; plot it directly, replacing the prior.
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
/// view state. The struct is deliberately float-free so `Hash`/`Eq`/`CaHash`
/// all derive cleanly (`CaHash` has no `f32` impl, and the app's `dyn Node`
/// requires `Hash`).
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.plot")]
pub struct Plot {
    /// Scope (scalar history) or Signal (plot a list).
    mode: PlotMode,
    /// Bars or line.
    style: PlotStyle,
    /// The maximum number of samples retained in [`PlotMode::Scope`].
    capacity: u32,
    /// Persisted body width (split from a `[u16; 2]` since that is not `CaHash`).
    width: u16,
    /// Persisted body height.
    height: u16,
    /// Line/bar colour. `None` follows the theme's strong text colour.
    color: Option<[u8; 4]>,
    /// Whether to draw the background grid.
    show_grid: bool,
    /// Whether to draw the axes.
    show_axes: bool,
    /// Whether the embedded plot accepts drag/zoom/scroll. Off by default to
    /// avoid clashing with the graph's own pan/zoom.
    interactive: bool,
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
            show_grid: true,
            show_axes: true,
            interactive: false,
        }
    }
}

/// Append `val` to the scope history `state`, dropping oldest entries so the
/// result holds at most `cap` items. Registered on the VM as `plot-push` and
/// called from the generated [`PlotMode::Scope`] expression.
///
/// `cap` is passed as an argument (not captured) so a single shared `plot-push`
/// serves every plot node with its own, always-current capacity.
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
    list.push_back(val);
    while list.len() > cap {
        list.pop_front();
    }
    SteelVal::ListV(list)
}

/// Read the node's stored series as `f64`s, skipping any non-numeric entries.
fn series(ctx: &NodeCtx) -> Vec<f64> {
    match ctx.extract_value() {
        Ok(Some(SteelVal::ListV(list))) => list.iter().filter_map(steel_num).collect(),
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
                // Append the incoming number to the bounded history.
                PlotMode::Scope => format!(
                    "(begin (if (number? {val}) (set! state (plot-push state {val} {cap})) void) {val})",
                    cap = self.capacity,
                ),
                // Store the incoming list directly.
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
        // Stateless and idempotent: re-registering the same name is a harmless
        // overwrite shared across all plot nodes.
        ctx.vm().register_fn("plot-push", plot_push);
    }
}

impl NodeUi for Plot {
    fn name(&self, _: &dyn Registry) -> &str {
        "plot"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Plot incoming values as a scrolling scope or a signal/array")
    }

    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        let style = uictx.style();
        let interaction = uictx.interaction();

        // A minimal extreme-bg frame, like `inspect`.
        let mut frame = egui_graph::node::default_frame(style, interaction);
        frame.fill = style.visuals.extreme_bg_color;

        let node_egui_id = uictx.egui_id();
        let resize_id = node_egui_id.with("resize");
        let plot_id = node_egui_id.with("plot");
        let min_size = egui::Vec2::splat(style.interaction.interact_radius * 2.0);
        let default_size = egui::vec2(self.width as f32, self.height as f32);

        // Read the series once, up-front (only borrows `ctx`).
        let ys = series(&ctx);

        uictx.framed_with(frame, |ui, _sockets| {
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

                    // Persist the resized body size. Like every field here it
                    // is content-addressed, so a resize is a real edit.
                    self.width = avail.x.max(min_size.x).round() as u16;
                    self.height = avail.y.max(min_size.y).round() as u16;

                    // Resolve config into locals so no borrow of `self` is held
                    // across the plot-building closure below.
                    let plot_style = self.style;
                    let show_grid = self.show_grid;
                    let show_axes = self.show_axes;
                    let interactive = self.interactive;
                    let color = match self.color {
                        Some([r, g, b, a]) => egui::Color32::from_rgba_unmultiplied(r, g, b, a),
                        None => ui.visuals().strong_text_color(),
                    };

                    let plot = egui_plot::Plot::new(plot_id)
                        .width(avail.x)
                        .height(avail.y)
                        .show_background(false)
                        .show_axes(egui::Vec2b::new(show_axes, show_axes))
                        .show_grid(egui::Vec2b::new(show_grid, show_grid))
                        .allow_drag(interactive)
                        .allow_zoom(interactive)
                        .allow_scroll(interactive)
                        .allow_boxed_zoom(interactive);

                    plot.show(ui, |plot_ui| match plot_style {
                        PlotStyle::Bars => {
                            let bars = ys
                                .iter()
                                .enumerate()
                                .map(|(i, &y)| egui_plot::Bar::new(i as f64, y).width(1.0))
                                .collect();
                            plot_ui.bar_chart(egui_plot::BarChart::new("", bars).color(color));
                        }
                        PlotStyle::Line => {
                            let points = egui_plot::PlotPoints::from_ys_f64(&ys);
                            plot_ui.line(egui_plot::Line::new("", points).color(color));
                        }
                    })
                    .response
                })
        })
    }

    fn inspector_rows(&mut self, _ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());

        // Mode.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.add(egui::Label::new("mode").selectable(false))
                    .on_hover_text("scope: scrolling history of a number; signal: plot a list");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.mode, PlotMode::Scope, "scope");
                    ui.selectable_value(&mut self.mode, PlotMode::Signal, "signal");
                });
            });
        });

        // Style.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.add(egui::Label::new("style").selectable(false))
                    .on_hover_text("how the series is drawn");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.style, PlotStyle::Bars, "bars");
                    ui.selectable_value(&mut self.style, PlotStyle::Line, "line");
                });
            });
        });

        // Capacity (scope history length).
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.add(egui::Label::new("capacity").selectable(false))
                    .on_hover_text("max samples retained in scope mode");
            });
            row.col(|ui| {
                let mut n = self.capacity as i32;
                if ui
                    .add(egui::DragValue::new(&mut n).range(1..=4096).speed(1.0))
                    .changed()
                {
                    self.capacity = n.clamp(1, 4096) as u32;
                }
            });
        });

        // Colour (with a reset-to-theme button when overridden).
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.add(egui::Label::new("colour").selectable(false))
                    .on_hover_text("line/bar colour; defaults to the theme");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    let mut col = match self.color {
                        Some([r, g, b, a]) => egui::Color32::from_rgba_unmultiplied(r, g, b, a),
                        None => ui.visuals().strong_text_color(),
                    };
                    if ui.color_edit_button_srgba(&mut col).changed() {
                        self.color = Some([col.r(), col.g(), col.b(), col.a()]);
                    }
                    if self.color.is_some() && ui.button("theme").clicked() {
                        self.color = None;
                    }
                });
            });
        });

        // Grid / axes / interactive toggles.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.add(egui::Label::new("display").selectable(false));
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.show_grid, "grid");
                    ui.checkbox(&mut self.show_axes, "axes");
                    ui.checkbox(&mut self.interactive, "interactive")
                        .on_hover_text("allow drag/zoom inside the node");
                });
            });
        });

        // Size readout.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.add(egui::Label::new("size").selectable(false));
            });
            row.col(|ui| {
                ui.label(format!("{} x {}", self.width, self.height));
            });
        });
    }

    fn context_menu(&mut self, ctx: &mut NodeCtx, ui: &mut egui::Ui) {
        if ui
            .button("clear history")
            .on_hover_text("empty the plotted series")
            .clicked()
        {
            ctx.update_value(SteelVal::ListV(Default::default())).ok();
            ui.close();
        }
    }

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => SocketDoc::ty("number | list").with_description(
                "scope: a number appended to the history; signal: a list of numbers to plot",
            ),
            SocketKind::Output => {
                SocketDoc::ty("any").with_description("the input value, unchanged")
            }
        })
    }
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
        let ep = entrypoint::push(
            vec![ix],
            g[petgraph::graph::NodeIndex::new(ix)].n_outputs(ctx) as u8,
        );
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

    // Scope mode appends each pushed number and bounds the history to `capacity`.
    #[test]
    fn scope_accumulates_bounded_history() {
        let mut g = petgraph::graph::DiGraph::new();
        let src = gantz_core::node::expr("5").unwrap().with_push_eval();
        let plot = Plot {
            mode: PlotMode::Scope,
            capacity: 3,
            ..Default::default()
        };
        let src = g.add_node(Box::new(src) as Box<dyn Node>);
        let plt = g.add_node(Box::new(plot) as Box<dyn Node>);
        g.add_edge(src, plt, Edge::from((0, 0)));

        let mut vm = vm_for(&g);
        // Five pushes, capacity three: history holds the most recent three.
        fire(&mut vm, &g, src.index(), 5);

        assert_eq!(list_of(&vm, plt.index()), vec![5.0, 5.0, 5.0]);
    }

    // Signal mode stores the incoming list verbatim, preserving order.
    #[test]
    fn signal_stores_list() {
        let mut g = petgraph::graph::DiGraph::new();
        let src = gantz_core::node::expr("(list 1 2 3)")
            .unwrap()
            .with_push_eval();
        let plot = Plot {
            mode: PlotMode::Signal,
            ..Default::default()
        };
        let src = g.add_node(Box::new(src) as Box<dyn Node>);
        let plt = g.add_node(Box::new(plot) as Box<dyn Node>);
        g.add_edge(src, plt, Edge::from((0, 0)));

        let mut vm = vm_for(&g);
        fire(&mut vm, &g, src.index(), 1);

        assert_eq!(list_of(&vm, plt.index()), vec![1.0, 2.0, 3.0]);
    }
}
