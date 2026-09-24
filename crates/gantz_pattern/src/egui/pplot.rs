//! A node that plots a pattern over a span of cycles.
//!
//! Each evaluation queries the input pattern with `pat/plot-data` and
//! stores the result as node state. The body draws that state against
//! cycle time and never calls into the VM. Discrete events draw as
//! horizontal segments at their values, with a dot at each onset.
//! Continuous signals draw as a line sampled across the span.

use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::SteelVal;
use gantz_egui::node::{F32, PlotLook};
use gantz_egui::ui_tree::plot::{resolve_color, show_plot, steel_num, y_bounds};
use gantz_egui::widget::node_inspector;
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

/// The decoded plot state.
#[derive(Debug, Default, PartialEq)]
struct PlotData {
    /// The plotted span, `[start, end]`, in cycles.
    span: [f64; 2],
    /// One per discrete event.
    segments: Vec<Segment>,
    /// The `[x, y]` samples of continuous signals.
    points: Vec<[f64; 2]>,
}

/// A discrete event's active part.
#[derive(Debug, PartialEq)]
struct Segment {
    start: f64,
    end: f64,
    value: f64,
    onset: bool,
}

/// The stroke width of an event segment, in points.
const SEGMENT_WIDTH: f32 = 2.0;
/// The radius of an onset dot, in points.
const ONSET_RADIUS: f32 = 3.0;
/// The space kept between the data and each fitted plot edge, in points.
/// It fits an onset dot plus a point of anti-aliasing.
const EDGE_PAD: f32 = ONSET_RADIUS + 1.0;

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

impl Default for Pplot {
    fn default() -> Self {
        Self {
            start: Ratio::from_integer(0),
            end: Ratio::from_integer(1),
            res: Res::Fixed(Res::DEFAULT_FIXED),
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
        self.look.body_ui(uictx, |ui, look| {
            let size = ui.available_size();
            draw(look, &data, plot_id, size, ui)
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
        let r = draw(&self.look, &data, ui.id().with("pplot"), size, ui);
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
        let mut summary = format!("{} events", data.segments.len());
        if !data.points.is_empty() {
            summary.push_str(&format!(" · {} signal samples", data.points.len()));
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

fn to_f64(r: Ratio<i64>) -> f64 {
    *r.numer() as f64 / *r.denom() as f64
}

/// Draw the plot data filling `size`.
fn draw(
    look: &PlotLook,
    data: &PlotData,
    plot_id: egui::Id,
    size: egui::Vec2,
    ui: &mut egui::Ui,
) -> egui::Response {
    let color = resolve_color(look.color, ui);
    let frame = look.frame();
    let values = data
        .segments
        .iter()
        .map(|s| s.value)
        .chain(data.points.iter().map(|p| p[1]));
    // Fitted edges are padded so onset dots at the extremes draw whole.
    // Fixed value bounds stay exact.
    let (ylo, yhi) = y_bounds(values, false, None, None);
    let ypad = edge_pad(ylo, yhi, size.y);
    let ylo = look.y_min.map_or(ylo - ypad, |v| f64::from(v.get()));
    let yhi = look.y_max.map_or(yhi + ypad, |v| f64::from(v.get()));
    let [xlo, xhi] = data.span;
    let xhi = xhi.max(xlo + f64::EPSILON);
    let xpad = edge_pad(xlo, xhi, size.x);
    let bounds = ([xlo - xpad, ylo], [xhi + xpad, yhi]);
    show_plot(frame, plot_id, size, bounds, ui, |plot_ui| {
        for s in &data.segments {
            let pts = vec![[s.start, s.value], [s.end, s.value]];
            plot_ui.line(
                egui_plot::Line::new("", pts)
                    .color(color)
                    .width(SEGMENT_WIDTH)
                    .allow_hover(frame.interactive),
            );
        }
        let onsets: Vec<[f64; 2]> = data
            .segments
            .iter()
            .filter(|s| s.onset)
            .map(|s| [s.start, s.value])
            .collect();
        if !onsets.is_empty() {
            plot_ui.points(
                egui_plot::Points::new("", onsets)
                    .color(color)
                    .radius(ONSET_RADIUS)
                    .filled(true)
                    .allow_hover(frame.interactive),
            );
        }
        if !data.points.is_empty() {
            plot_ui.line(
                egui_plot::Line::new("", data.points.clone())
                    .color(color)
                    .allow_hover(frame.interactive),
            );
        }
    })
}

/// The padding in plot units, on each side of `lo..hi` drawn over `len`
/// points, that leaves [`EDGE_PAD`] points between the data and each edge.
/// Zero when `len` leaves no room for it.
fn edge_pad(lo: f64, hi: f64, len: f32) -> f64 {
    let room = f64::from(len) - 2.0 * f64::from(EDGE_PAD);
    if room > 0.0 {
        f64::from(EDGE_PAD) * (hi - lo) / room
    } else {
        0.0
    }
}

/// Read and decode the node's stored plot data. Empty when absent or
/// malformed.
fn plot_data_of(ctx: &NodeCtx) -> PlotData {
    match ctx.extract_value() {
        Ok(Some(val)) => plot_data(&val).unwrap_or_default(),
        _ => PlotData::default(),
    }
}

/// Decode `pat/plot-data` output. `None` unless the value is the expected
/// `(start end segments points)` list. Malformed entries are skipped.
fn plot_data(val: &SteelVal) -> Option<PlotData> {
    let [start, end, segments, points] = list_n(val)?;
    let span = [steel_num(&start)?, steel_num(&end)?];
    let segments = list(&segments)?
        .iter()
        .filter_map(|seg| {
            let [start, end, value, onset] = list_n(seg)?;
            Some(Segment {
                start: steel_num(&start)?,
                end: steel_num(&end)?,
                value: steel_num(&value)?,
                onset: matches!(onset, SteelVal::BoolV(true)),
            })
        })
        .collect();
    let points = list(&points)?
        .iter()
        .filter_map(|pt| {
            let [x, y] = list_n(pt)?;
            Some([steel_num(&x)?, steel_num(&y)?])
        })
        .collect();
    Some(PlotData {
        span,
        segments,
        points,
    })
}

/// The elements of a list value.
fn list(val: &SteelVal) -> Option<Vec<SteelVal>> {
    match val {
        SteelVal::ListV(l) => Some(l.iter().cloned().collect()),
        _ => None,
    }
}

/// The elements of a list value of exactly `N` elements.
fn list_n<const N: usize>(val: &SteelVal) -> Option<[SteelVal; N]> {
    list(val)?.try_into().ok()
}

#[cfg(test)]
mod tests {
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

    fn seg(start: f64, end: f64, value: f64, onset: bool) -> Segment {
        Segment {
            start,
            end,
            value,
            onset,
        }
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
        assert_eq!(data.segments, vec![seg(0.5, 1.0, 2.0, true)]);
        assert!(data.points.is_empty());
    }

    // A connected number plots from 0 to that many cycles.
    #[test]
    fn connected_number_span_overrides() {
        let (g, plot) = graph(Pplot::default(), Some("2"));
        let data = eval(&g, plot);
        assert_eq!(data.span, [0.0, 2.0]);
        assert_eq!(
            data.segments,
            vec![
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

    // The pad leaves `EDGE_PAD` points at each edge. With no room for it,
    // there is none.
    #[test]
    fn edge_pad_fits_onset_dots() {
        let len = 100.0;
        let pad = edge_pad(0.0, 1.0, len);
        let pts_per_unit = f64::from(len) / (1.0 + 2.0 * pad);
        assert!((pad * pts_per_unit - f64::from(EDGE_PAD)).abs() < 1e-9);
        assert_eq!(edge_pad(0.0, 1.0, 2.0 * EDGE_PAD), 0.0);
    }

    // Decoding rejects a malformed state and skips malformed entries.
    #[test]
    fn plot_data_decodes_totally() {
        let num = |n: f64| SteelVal::NumV(n);
        let list = |xs: Vec<SteelVal>| SteelVal::ListV(xs.into_iter().collect());
        assert_eq!(plot_data(&list(vec![])), None);
        assert_eq!(plot_data(&num(1.0)), None);
        let state = list(vec![
            num(0.0),
            num(1.0),
            list(vec![
                list(vec![num(0.0), num(1.0), num(3.0), SteelVal::BoolV(true)]),
                list(vec![num(0.0)]),
            ]),
            list(vec![list(vec![num(0.5), num(0.25)]), num(9.0)]),
        ]);
        assert_eq!(
            plot_data(&state),
            Some(PlotData {
                span: [0.0, 1.0],
                segments: vec![seg(0.0, 1.0, 3.0, true)],
                points: vec![[0.5, 0.25]],
            }),
        );
    }
}
