//! A node that plots a pattern over a span of cycles.
//!
//! Each evaluation queries the input pattern with `pat/plot-data` and
//! stores the result as node state. The body draws that state against
//! cycle time and never calls into the VM. Numeric values draw as
//! horizontal segments with a dot at each onset. Continuous signals draw as
//! a line sampled across the span. Other values draw as boxes over their
//! active spans, labelled for strings and symbols. Map and list values split
//! into one channel per key or index, stacked in one plot or expanded into
//! one plot each.

mod data;
mod draw;
pub(crate) mod sugar;

use data::{PlotData, plot_data};
use draw::DrawConf;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::SteelVal;
use gantz_egui::node::{F32, PlotLook};
use gantz_egui::ui_tree::plot::resolve_color;
use gantz_egui::widget::node_inspector::{self, radio_option};
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, NodeViewResponse, SocketDoc,
    SocketKind,
};
use gantz_nodetag::NodeTag;
use num_rational::Ratio;
use serde::{Deserialize, Serialize};

/// A node that plots the pattern it receives over a span of cycles.
///
/// Every field feeds the content address, so each inspector edit is a real,
/// undoable change. See the `changed` contract on [`gantz_egui::NodeUi`].
#[derive(Clone, Debug, Hash, Deserialize, Serialize, NodeTag)]
pub struct Pplot {
    /// The start of the plotted span in cycles, when no span is connected.
    start: Ratio<i64>,
    /// The end of the plotted span in cycles, when no span is connected.
    end: Ratio<i64>,
    /// The number of slices a continuous signal is sampled over.
    res: Res,
    /// How map and list value channels are laid out.
    layout: ValueLayout,
    /// Colours for channels by key text, or by decimal index for lists.
    /// Other channels use the look's colour.
    key_colors: Vec<KeyColor>,
    /// The body appearance, shared with the `plot` node.
    #[serde(flatten)]
    look: PlotLook,
}

/// How many slices a continuous signal is sampled over.
#[derive(Clone, Copy, Debug, Hash, Deserialize, Serialize)]
pub enum Res {
    /// A fixed slice count.
    Fixed(u16),
    /// One slice per this many points of the committed body width, so the
    /// sampling fits the plot as it is resized.
    Fit(F32),
}

/// How the channels of map and list values are laid out.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub enum ValueLayout {
    /// Every channel in one plot.
    #[default]
    Stack,
    /// One plot per channel, in key order.
    Expand,
}

/// A colour for the channel whose key text matches `key`.
#[derive(Clone, Debug, Hash, PartialEq, Deserialize, Serialize)]
pub struct KeyColor {
    /// A map key's text, or a list index in decimal.
    pub key: String,
    /// The channel colour.
    pub color: [u8; 4],
}

impl Res {
    /// The default fixed slice count.
    pub const DEFAULT_FIXED: u16 = 128;
    /// The default points per slice when fitting.
    pub const DEFAULT_FIT: f32 = 2.0;
    /// The maximum slice count.
    const MAX: u16 = 4096;
    /// The range of points per slice when fitting.
    const FIT_RANGE: std::ops::RangeInclusive<f32> = 0.5..=64.0;

    /// The slice count for a body `width` points wide.
    fn slices(self, width: u16) -> u16 {
        let n = match self {
            Res::Fixed(n) => n,
            Res::Fit(pts) => {
                let pts = pts
                    .get()
                    .clamp(*Self::FIT_RANGE.start(), *Self::FIT_RANGE.end());
                (f32::from(width) / pts).ceil() as u16
            }
        };
        n.clamp(1, Self::MAX)
    }
}

impl Pplot {
    fn draw_conf(&self) -> DrawConf<'_> {
        DrawConf {
            look: &self.look,
            layout: self.layout,
            key_colors: &self.key_colors,
        }
    }
}

impl Default for Pplot {
    fn default() -> Self {
        Self {
            start: Ratio::from_integer(0),
            end: Ratio::from_integer(1),
            res: Res::Fixed(Res::DEFAULT_FIXED),
            layout: ValueLayout::default(),
            key_colors: vec![],
            look: PlotLook::default(),
        }
    }
}

impl gantz_core::Node for Pplot {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        2
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        let default = format!("(pat/span {} {})", self.start, self.end);
        let inputs = ctx.inputs();
        let expr = match inputs.first() {
            Some(Some(p)) => {
                let span = match inputs.get(1) {
                    Some(Some(span)) => format!("(pat/as-span {span} {default})"),
                    _ => default,
                };
                format!(
                    "(begin (set! state (pat/plot-data {p} {span} {res})) {p})",
                    res = self.res.slices(self.look.width),
                )
            }
            _ => "(begin state)".to_string(),
        };
        node::parse_expr(&expr)
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        node::state::init_value_if_absent(ctx.vm(), path, || SteelVal::ListV(Default::default()))
            .unwrap();
    }

    fn required_modules(&self, _ctx: MetaCtx) -> Vec<String> {
        vec![crate::MODULE.name.to_string()]
    }
}

impl NodeUi for Pplot {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "pplot".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Plot a pattern's events over a span of cycles")
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let plot_id = uictx.egui_id().with("pplot");
        let data = plot_data_of(&ctx);
        let (layout, key_colors) = (self.layout, &self.key_colors);
        self.look.body_ui(uictx, |ui, look| {
            let conf = DrawConf {
                look,
                layout,
                key_colors,
            };
            let size = ui.available_size();
            draw::draw(conf, &data, plot_id, size, ui)
        })
    }

    fn view_no_margin(&self) -> bool {
        true
    }

    fn view_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
        // The detached view fills the pane and never writes back the size.
        // The id derives from `ui`, which the caller scopes per pane.
        let data = plot_data_of(&ctx);
        let size = ui.available_size();
        let plot_id = ui.id().with("pplot");
        let r = draw::draw(self.draw_conf(), &data, plot_id, size, ui);
        let mut out = NodeViewResponse::default();
        out.inner = Some(r);
        out
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        let mut changed = false;

        // A summary in place of the suppressed default state row.
        let data = plot_data_of(ctx);
        let mut summary = format!("{} events", data.n_events());
        if data.channels.len() > 1 {
            summary.push_str(&format!(" · {} channels", data.channels.len()));
        }
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
                ui.label("span");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    changed |= span_edit(ui, &mut self.start, &mut self.end);
                });
            });
        });

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("res");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    changed |= res_edit(ui, &mut self.res);
                });
            });
        });

        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("values");
            });
            row.col(|ui| {
                ui.horizontal(|ui| {
                    changed |= radio_option(
                        ui,
                        &mut self.layout,
                        ValueLayout::Stack,
                        "stack",
                        "plot map and list channels together",
                    );
                    changed |= radio_option(
                        ui,
                        &mut self.layout,
                        ValueLayout::Expand,
                        "expand",
                        "plot each map key or list index on its own",
                    );
                });
            });
        });

        // One line per key colour, plus the add button.
        let keys_h = row_h * (self.key_colors.len() + 1) as f32;
        let keys_id = egui::Id::new("pplot_keys").with(ctx.path());
        let base = self.look.color;
        body.row(keys_h, |mut row| {
            row.col(|ui| {
                ui.label("keys");
            });
            row.col(|ui| {
                changed |= keys_edit(ui, keys_id, &mut self.key_colors, base);
            });
        });

        changed |= self.look.inspector_rows(body);

        let mut resp = InspectorRowsResponse::default();
        resp.set_changed(changed);
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        Some(match (kind, ix) {
            (SocketKind::Input, 0) => {
                SocketDoc::ty("pattern").with_description("the pattern to plot")
            }
            (SocketKind::Input, _) => SocketDoc::ty("span or number").with_description(
                "the cycles to plot. a number n plots 0 to n. unconnected uses the inspector span",
            ),
            (SocketKind::Output, _) => {
                SocketDoc::ty("pattern").with_description("the input pattern, unchanged")
            }
        })
    }

    fn show_state(&self) -> bool {
        // The raw state is a long list. The inspector summarises it instead.
        false
    }
}

/// Edit the span bounds as floats snapped to the pattern grid. The end stays
/// at least one grid step after the start. Returns whether either changed.
fn span_edit(ui: &mut egui::Ui, start: &mut Ratio<i64>, end: &mut Ratio<i64>) -> bool {
    let step = Ratio::new(1, 1920);
    let mut changed = false;
    let mut s = to_f64(*start);
    let s_resp = ui
        .add(egui::DragValue::new(&mut s).speed(1.0 / 16.0))
        .on_hover_text("start, in cycles");
    if s_resp.changed()
        && let Some(r) = crate::mini::snap(s)
    {
        *start = r;
        *end = (*end).max(r + step);
        changed = true;
    }
    let mut e = to_f64(*end);
    let e_resp = ui
        .add(egui::DragValue::new(&mut e).speed(1.0 / 16.0))
        .on_hover_text("end, in cycles");
    if e_resp.changed()
        && let Some(r) = crate::mini::snap(e)
    {
        *end = r.max(*start + step);
        changed = true;
    }
    changed
}

/// Edit the signal resolution. A mode toggle picks a fixed slice count or a
/// fit to the body width, then a dialer edits that mode's value. Switching
/// mode starts from the mode's default. Returns whether it changed.
fn res_edit(ui: &mut egui::Ui, res: &mut Res) -> bool {
    let mut changed = false;
    let fixed = matches!(res, Res::Fixed(_));
    if ui
        .selectable_label(fixed, "fixed")
        .on_hover_text("sample a signal over a fixed number of slices")
        .clicked()
        && !fixed
    {
        *res = Res::Fixed(Res::DEFAULT_FIXED);
        changed = true;
    }
    if ui
        .selectable_label(!fixed, "fit")
        .on_hover_text("sample a signal once per this many points of the plot width")
        .clicked()
        && fixed
    {
        *res = Res::Fit(F32(Res::DEFAULT_FIT));
        changed = true;
    }
    changed |= match res {
        Res::Fixed(n) => ui
            .add(egui::DragValue::new(n).range(1..=Res::MAX).speed(1.0))
            .on_hover_text("slices a signal is sampled over")
            .changed(),
        Res::Fit(F32(pts)) => ui
            .add(
                egui::DragValue::new(pts)
                    .range(Res::FIT_RANGE)
                    .speed(0.1)
                    .suffix(" pt"),
            )
            .on_hover_text("points of plot width per signal slice")
            .changed(),
    };
    changed
}

/// Edit the key colours. Each line has a key, a colour and a remove button.
/// A final button adds a line in the look's colour. Returns whether any
/// changed.
fn keys_edit(
    ui: &mut egui::Ui,
    id: egui::Id,
    key_colors: &mut Vec<KeyColor>,
    base: Option<[u8; 4]>,
) -> bool {
    let mut changed = false;
    let mut remove = None;
    ui.vertical(|ui| {
        for (i, kc) in key_colors.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                changed |= key_edit(ui, id.with(i), &mut kc.key);
                let [r, g, b, a] = kc.color;
                let mut col = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
                if ui
                    .color_edit_button_srgba(&mut col)
                    .on_hover_text("the channel colour")
                    .changed()
                {
                    kc.color = [col.r(), col.g(), col.b(), col.a()];
                    changed = true;
                }
                if ui
                    .small_button("x")
                    .on_hover_text("remove this key colour")
                    .clicked()
                {
                    remove = Some(i);
                }
            });
        }
        if ui
            .small_button("+")
            .on_hover_text("colour a map key or list index")
            .clicked()
        {
            let col = resolve_color(base, ui);
            key_colors.push(KeyColor {
                key: String::new(),
                color: [col.r(), col.g(), col.b(), col.a()],
            });
            changed = true;
        }
    });
    if let Some(i) = remove {
        key_colors.remove(i);
        changed = true;
    }
    changed
}

/// Edit a key colour's key. The text is buffered in egui temp memory while
/// focused and commits when focus is lost, so a keystroke does not commit a
/// content address. Returns whether the key changed.
fn key_edit(ui: &mut egui::Ui, id: egui::Id, key: &mut String) -> bool {
    let mut buf = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| key.clone());
    let resp = ui.add(
        egui::TextEdit::singleline(&mut buf)
            .desired_width(48.0)
            .hint_text("key"),
    );
    let changed = resp.lost_focus() && buf != *key;
    if changed {
        *key = buf.clone();
    }
    // Read before `data_mut`. It holds the context lock, which `has_focus`
    // also takes, and the lock is not re-entrant.
    let focused = resp.has_focus();
    ui.data_mut(|d| {
        if focused {
            d.insert_temp(id, buf);
        } else {
            d.remove::<String>(id);
        }
    });
    changed
}

fn to_f64(r: Ratio<i64>) -> f64 {
    *r.numer() as f64 / *r.denom() as f64
}

/// Read and decode the node's stored plot data. Empty when absent or
/// malformed.
fn plot_data_of(ctx: &NodeCtx) -> PlotData {
    match ctx.extract_value() {
        Ok(Some(val)) => plot_data(&val).unwrap_or_default(),
        _ => PlotData::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::data::{ChannelKey, Leaf, Seg};
    use super::*;
    use gantz_core::Edge;
    use gantz_core::compile::{entry_fn_name, push_pull_entrypoints};
    use gantz_core::node::{Node, WithPushEval};

    type Graph = petgraph::graph::DiGraph<Box<dyn Node>, Edge>;

    fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
        None
    }

    // A push source feeding `pmini "1 2"` into a pplot. With `span`, a second
    // push source feeds that expr into the pplot's span input.
    fn graph(pplot: Pplot, span: Option<&str>) -> (Graph, usize) {
        let mut g = Graph::new();
        let push = g.add_node(Box::new(node::expr("'()").unwrap().with_push_eval()));
        let pmini = g.add_node(Box::new(crate::Pmini::new("1 2")));
        let plot = g.add_node(Box::new(pplot));
        g.add_edge(push, pmini, Edge::from((0, 0)));
        g.add_edge(pmini, plot, Edge::from((0, 0)));
        if let Some(src) = span {
            let span = g.add_node(Box::new(node::expr(src).unwrap()));
            g.add_edge(push, span, Edge::from((0, 0)));
            g.add_edge(span, plot, Edge::from((0, 1)));
        }
        (g, plot.index())
    }

    // Evaluate the graph once from its push source and decode the plot state.
    fn eval(g: &Graph, plot: usize) -> PlotData {
        let eps = push_pull_entrypoints(&no_lookup, g);
        let (mut vm, _) = gantz_core::vm::init_with_modules(
            &no_lookup,
            g,
            &eps,
            &Default::default(),
            crate::modules(),
        )
        .expect("init");
        vm.call_function_by_name_with_args(&entry_fn_name(&eps[0].id()), vec![])
            .expect("eval");
        let state = node::state::extract_value(&vm, &[plot]).unwrap().unwrap();
        plot_data(&state).expect("plot data")
    }

    fn seg(start: f64, end: f64, value: f64, onset: bool) -> Seg {
        Seg {
            start,
            end,
            leaf: Leaf::Num(value),
            onset,
        }
    }

    fn whole(data: &PlotData) -> &[Seg] {
        &data.channels[&ChannelKey::Whole].segments
    }

    // An unconnected span input plots the node's own span.
    #[test]
    fn unconnected_span_uses_node_span() {
        let pplot = Pplot {
            start: Ratio::new(1, 2),
            ..Default::default()
        };
        let (g, plot) = graph(pplot, None);
        let data = eval(&g, plot);
        assert_eq!(data.span, [0.5, 1.0]);
        assert_eq!(whole(&data), [seg(0.5, 1.0, 2.0, true)]);
        assert!(data.channels[&ChannelKey::Whole].points.is_empty());
    }

    // A connected number plots from 0 to that many cycles.
    #[test]
    fn connected_number_span_overrides() {
        let (g, plot) = graph(Pplot::default(), Some("2"));
        let data = eval(&g, plot);
        assert_eq!(data.span, [0.0, 2.0]);
        assert_eq!(
            whole(&data),
            [
                seg(0.0, 0.5, 1.0, true),
                seg(0.5, 1.0, 2.0, true),
                seg(1.0, 1.5, 1.0, true),
                seg(1.5, 2.0, 2.0, true),
            ],
        );
    }

    // A connected non-span falls back to the node's span.
    #[test]
    fn connected_junk_span_falls_back() {
        let (g, plot) = graph(Pplot::default(), Some("\"x\""));
        assert_eq!(eval(&g, plot).span, [0.0, 1.0]);
    }

    // A map-valued pattern plots one channel per key, each with its leaves.
    #[test]
    fn map_values_split_by_key() {
        let mut g = Graph::new();
        let src = node::expr("(pat/pure (hash 's 'bd 'n 2))")
            .unwrap()
            .with_requires(["gantz/pattern"])
            .with_push_eval();
        let src = g.add_node(Box::new(src));
        let plot = g.add_node(Box::new(Pplot::default()));
        g.add_edge(src, plot, Edge::from((0, 0)));
        let data = eval(&g, plot.index());
        let keys: Vec<_> = data.channels.keys().cloned().collect();
        assert_eq!(
            keys,
            [ChannelKey::Key("n".into()), ChannelKey::Key("s".into())]
        );
        let leaf = |k: &str| {
            data.channels[&ChannelKey::Key(k.into())].segments[0]
                .leaf
                .clone()
        };
        assert_eq!(leaf("n"), Leaf::Num(2.0));
        assert_eq!(leaf("s"), Leaf::Label("bd".into()));
    }

    // The keys editor draws a key colour line without deadlocking the egui
    // context. It runs on a thread so a deadlock fails the test, not hangs it.
    #[test]
    fn keys_edit_draws_without_deadlock() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            let mut kcs = vec![KeyColor {
                key: String::new(),
                color: [1, 2, 3, 255],
            }];
            for _ in 0..2 {
                let _ = ctx.run_ui(Default::default(), |ui| {
                    keys_edit(ui, egui::Id::new("keys"), &mut kcs, None);
                });
            }
            tx.send(()).ok();
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("keys editor deadlocked");
    }

    // A fixed count is clamped. A fit derives the count from the body width,
    // rounding up.
    #[test]
    fn res_slices() {
        assert_eq!(Res::Fixed(64).slices(120), 64);
        assert_eq!(Res::Fixed(0).slices(120), 1);
        assert_eq!(Res::Fixed(u16::MAX).slices(120), Res::MAX);
        assert_eq!(Res::Fit(F32(2.0)).slices(120), 60);
        assert_eq!(Res::Fit(F32(7.0)).slices(120), 18);
        // Out-of-range densities clamp to the fit range.
        assert_eq!(Res::Fit(F32(0.0)).slices(120), 240);
        assert_eq!(Res::Fit(F32(1000.0)).slices(120), 2);
    }
}
