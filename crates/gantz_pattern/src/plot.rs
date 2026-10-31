//! The `pat.plot` node definition, which plots a pattern over a span of
//! cycles.
//!
//! Each evaluation queries the input pattern with `pat/plot-data`, and the
//! native fn `pat/plot-figures` lays the result out as the figures of a
//! `gantz_ui` canvas, with a summary of the events. The node stores both as
//! its state, with the data that they come from. An edit of the look lays
//! the data out again, so it shows before the next evaluation. The view of
//! the node is a canvas bound to the figures, so drawing never calls into
//! the VM.
//!
//! Numeric values draw as horizontal segments with a dot at each onset.
//! Continuous signals draw as a line sampled across the span. Other values
//! draw as boxes in lanes that divide the height of the plot, labelled for
//! strings and symbols. Map and list values split into one channel per key
//! or index, stacked in one plot or expanded into one plot each.

use gantz_core::datum;
use gantz_core::steel::gc::Gc;
use gantz_core::steel::rerrs::ErrorKind;
use gantz_core::steel::steel_vm::{builtin::BuiltInModule, register_fn::RegisterFn};
use gantz_core::steel::{SteelErr, SteelVal};
use gantz_core::vm::SteelModule;
use gantz_format::{Datum, FormatError, SugarArgs};
use gantz_nodedef::{ExprArgs, Meta, NodeDef, Resize, Row, SocketDoc, View};
use gantz_ui::{
    At, BindPath, Bound, Button, Choice, Color, Dialer, Edit, Element, Field, Label, Rgba,
};
use num_rational::Ratio;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A node that plots the pattern it receives over a span of cycles.
///
/// Its stored form must not change, since stored graphs address their nodes
/// by it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
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
    /// The body appearance, shared in form with the `plot` node.
    #[serde(flatten)]
    look: Look,
}

/// How many slices a continuous signal is sampled over.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
enum Res {
    /// A fixed slice count.
    Fixed(u16),
    /// One slice per this many points of the body width, so the sampling
    /// fits the plot as it is resized.
    Fit(f32),
}

/// How the channels of map and list values are laid out.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
enum ValueLayout {
    /// Every channel in one plot.
    #[default]
    Stack,
    /// One plot per channel, in key order.
    Expand,
}

/// A colour for the channel whose key text matches `key`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct KeyColor {
    /// A map key's text, or a list index in decimal.
    key: String,
    /// The channel colour.
    color: [u8; 4],
}

/// The body appearance of a plot.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct Look {
    /// The body width.
    width: u16,
    /// The body height.
    height: u16,
    /// The colour of the data. `None` follows the theme.
    color: Option<[u8; 4]>,
    show_grid: bool,
    show_axes: bool,
    /// Whether hovering shows a crosshair and the value beneath it.
    interactive: bool,
    /// Whether the data is inset within the margin of the node frame.
    margin: bool,
    /// A fixed lower bound for the value axis.
    y_min: Option<f32>,
    /// A fixed upper bound for the value axis.
    y_max: Option<f32>,
}

/// A plottable value.
#[derive(Clone, Debug, PartialEq)]
enum Leaf {
    /// A number, or a bool as 1 or 0. Plotted on the value axis.
    Num(f64),
    /// A string, symbol or char. Plotted as a labelled span.
    Label(String),
    /// Any other value, with its display text. Plotted as a bare span.
    Opaque(String),
}

/// The channel that an event value, or a part of one, plots in.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ChannelKey {
    /// A whole value that is neither a map nor a list.
    Whole,
    /// An element of a list or vector value.
    Index(usize),
    /// An entry of a map value, by the key's text.
    Key(String),
}

/// A discrete event's active part in one channel.
#[derive(Clone, Debug, PartialEq)]
struct Seg {
    start: f64,
    end: f64,
    leaf: Leaf,
    onset: bool,
}

/// One channel's events and signal samples.
#[derive(Clone, Debug, Default, PartialEq)]
struct Channel {
    segments: Vec<Seg>,
    /// The `[x, y]` samples of continuous numeric signals.
    points: Vec<[f64; 2]>,
}

/// The decoded output of `pat/plot-data`.
#[derive(Clone, Debug, Default, PartialEq)]
struct PlotData {
    span: [f64; 2],
    channels: BTreeMap<ChannelKey, Channel>,
}

/// How the figures of a plot draw, as `pat/plot-figures` reads it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Conf {
    layout: ValueLayout,
    color: Option<String>,
    key_colors: Vec<(String, String)>,
    y_min: Option<f64>,
    y_max: Option<f64>,
}

/// The keyword of the plot node, also its name in the palette.
pub const KEYWORD: &str = "pat.plot";

/// The keyword from before the `pat.` namespace.
pub(crate) const OLD_KEYWORD: &str = "pplot";

/// The Steel module of `pat/plot-figures`.
pub const MODULE: SteelModule = SteelModule::new(
    "gantz/pattern-plot",
    "(require-builtin #%gantz/pattern-plot)\n(provide pat/plot-figures)\n",
)
.with_builtin(builtin);

/// The radius of an onset dot in the canvas, plus a point of anti-aliasing:
/// the points kept between the data and each edge.
const EDGE_PAD: f64 = 4.0;
/// The fraction of a lane's height left as a gap above and below its boxes.
const LANE_GAP: f64 = 0.1;
/// The step of the grid that span bounds snap to, in cycles.
const SPAN_STEP: Ratio<i64> = Ratio::new_raw(1, 1920);
/// The colour of a new key colour when the look follows the theme. A node
/// definition does not see the theme, so it is a grey that shows on light and
/// dark themes.
const KEY_COLOR: [u8; 4] = [128, 128, 128, 255];
/// The key of the plot data in the state of the node.
const DATA: &str = "data";
/// The key of the figures in the state of the node.
const FIGURES: &str = "figures";
/// The key of the summary of the events in the state of the node.
const SUMMARY: &str = "summary";

impl Res {
    /// The default fixed slice count.
    const DEFAULT_FIXED: u16 = 128;
    /// The default points per slice when fitting.
    const DEFAULT_FIT: f32 = 2.0;
    /// The maximum slice count.
    const MAX: u16 = 4096;
    /// The range of points per slice when fitting.
    const FIT_RANGE: std::ops::RangeInclusive<f32> = 0.5..=64.0;

    /// The slice count for a body `width` points wide.
    fn slices(self, width: u16) -> u16 {
        let n = match self {
            Res::Fixed(n) => n,
            Res::Fit(pts) => {
                let pts = pts.clamp(*Self::FIT_RANGE.start(), *Self::FIT_RANGE.end());
                (f32::from(width) / pts).ceil() as u16
            }
        };
        n.clamp(1, Self::MAX)
    }
}

impl Look {
    /// The default body size, `[width, height]`.
    const DEFAULT_SIZE: [u16; 2] = [120, 80];
}

impl Conf {
    /// The conf as a quoted Steel datum, for `pat/plot-figures`.
    fn expr(&self) -> String {
        let text = |t: &Option<String>| t.as_ref().map_or("#f".to_string(), |t| format!("{t:?}"));
        let layout = match self.layout {
            ValueLayout::Stack => "stack",
            ValueLayout::Expand => "expand",
        };
        let keys: Vec<String> = self
            .key_colors
            .iter()
            .map(|(key, color)| format!("({key:?} {color:?})"))
            .collect();
        let bound = |b: Option<f64>| b.map_or("#f".to_string(), |b| format!("{b:?}"));
        format!(
            "(quote ({layout} {} ({}) {} {}))",
            text(&self.color),
            keys.join(" "),
            bound(self.y_min),
            bound(self.y_max),
        )
    }
}

impl Pplot {
    /// How the figures of the node draw.
    fn conf(&self) -> Conf {
        Conf {
            layout: self.layout,
            color: self.look.color.map(|c| Rgba(c).hex()),
            key_colors: self
                .key_colors
                .iter()
                .map(|kc| (kc.key.clone(), Rgba(kc.color).hex()))
                .collect(),
            y_min: self.look.y_min.map(f64::from),
            y_max: self.look.y_max.map(f64::from),
        }
    }

    /// Applies the edit `edit` with `value`. Gives whether the node changed.
    fn apply(&mut self, edit: &str, value: &Datum) -> bool {
        let before = self.clone();
        match (edit, value) {
            ("start", v) => {
                if let Some(r) = number(v).and_then(crate::mini::snap) {
                    self.start = r;
                    self.end = self.end.max(r + SPAN_STEP);
                }
            }
            ("end", v) => {
                if let Some(r) = number(v).and_then(crate::mini::snap) {
                    self.end = r.max(self.start + SPAN_STEP);
                }
            }
            ("res-mode", Datum::Str(mode)) => match (mode.as_str(), self.res) {
                ("fixed", Res::Fit(_)) => self.res = Res::Fixed(Res::DEFAULT_FIXED),
                ("fit", Res::Fixed(_)) => self.res = Res::Fit(Res::DEFAULT_FIT),
                _ => (),
            },
            ("res", v) => {
                if let Some(n) = number(v) {
                    self.res = match self.res {
                        Res::Fixed(_) => Res::Fixed(n.clamp(1.0, f64::from(Res::MAX)) as u16),
                        Res::Fit(_) => {
                            let (lo, hi) = (*Res::FIT_RANGE.start(), *Res::FIT_RANGE.end());
                            Res::Fit((n as f32).clamp(lo, hi))
                        }
                    };
                }
            }
            ("layout", Datum::Str(layout)) => match layout.as_str() {
                "stack" => self.layout = ValueLayout::Stack,
                "expand" => self.layout = ValueLayout::Expand,
                _ => (),
            },
            ("key-add", _) => self.key_colors.push(KeyColor {
                key: String::new(),
                color: self.look.color.unwrap_or(KEY_COLOR),
            }),
            ("color", Datum::Str(c)) => {
                self.look.color = Rgba::parse(c).map(|c| c.0).or(self.look.color)
            }
            ("color", _) => self.look.color = None,
            ("y-min", v) => self.look.y_min = number(v).map(|n| n as f32),
            ("y-max", v) => self.look.y_max = number(v).map(|n| n as f32),
            (edit, v) => {
                let Some((what, ix)) = edit.rsplit_once('-') else {
                    return false;
                };
                let Some(ix) = ix
                    .parse::<usize>()
                    .ok()
                    .filter(|&i| i < self.key_colors.len())
                else {
                    return false;
                };
                match (what, v) {
                    ("key-name", Datum::Str(key)) => self.key_colors[ix].key = key.clone(),
                    ("key-color", Datum::Str(c)) => {
                        if let Some(c) = Rgba::parse(c).map(|c| c.0) {
                            self.key_colors[ix].color = c;
                        }
                    }
                    ("key-remove", _) => {
                        self.key_colors.remove(ix);
                    }
                    _ => (),
                }
            }
        }
        *self != before
    }

    /// The rows of the node inspector.
    fn rows(&self) -> Vec<Row> {
        let row = |label: &str, ui: Element, lines: usize| Row {
            label: label.into(),
            ui,
            lines: lines as u32,
        };
        let res = match self.res {
            Res::Fixed(n) => Dialer {
                on: Some(Edit("res".into())),
                value: Some(f64::from(n)),
                min: Some(1.0),
                max: Some(f64::from(Res::MAX)),
                step: Some(1.0),
                tip: Some("slices a signal is sampled over".into()),
                ..Dialer::default()
            },
            Res::Fit(pts) => Dialer {
                on: Some(Edit("res".into())),
                value: Some(f64::from(pts)),
                min: Some(f64::from(*Res::FIT_RANGE.start())),
                max: Some(f64::from(*Res::FIT_RANGE.end())),
                step: Some(0.1),
                suffix: Some(" pt".into()),
                tip: Some("points of plot width per signal slice".into()),
                ..Dialer::default()
            },
        };
        let res_mode = Choice {
            on: Some(Edit("res-mode".into())),
            value: Some(
                match self.res {
                    Res::Fixed(_) => "fixed",
                    Res::Fit(_) => "fit",
                }
                .into(),
            ),
            options: vec!["fixed".into(), "fit".into()],
            tips: vec![
                "sample a signal over a fixed number of slices".into(),
                "sample a signal once per this many points of the plot width".into(),
            ],
            ..Choice::default()
        };
        let layout = Choice {
            on: Some(Edit("layout".into())),
            value: Some(
                match self.layout {
                    ValueLayout::Stack => "stack",
                    ValueLayout::Expand => "expand",
                }
                .into(),
            ),
            options: vec!["stack".into(), "expand".into()],
            tips: vec![
                "plot map and list channels together".into(),
                "plot each map key or list index on its own".into(),
            ],
            ..Choice::default()
        };
        let mut keys: Vec<Element> = self
            .key_colors
            .iter()
            .enumerate()
            .map(|(i, kc)| {
                hrow(vec![
                    Element::TextInput(gantz_ui::TextInput {
                        on: Some(Edit(format!("key-name-{i}"))),
                        value: Some(kc.key.clone()),
                        hint: Some("key".into()),
                        ..gantz_ui::TextInput::default()
                    }),
                    Element::Color(Color {
                        on: Some(Edit(format!("key-color-{i}"))),
                        value: Some(Rgba(kc.color)),
                        theme: false,
                        tip: Some("the channel colour".into()),
                        ..Color::default()
                    }),
                    Element::Button(Button {
                        on: Some(Edit(format!("key-remove-{i}"))),
                        label: Some("x".into()),
                        tip: Some("remove this key colour".into()),
                        ..Button::default()
                    }),
                ])
            })
            .collect();
        keys.push(Element::Button(Button {
            on: Some(Edit("key-add".into())),
            label: Some("+".into()),
            tip: Some("colour a map key or list index".into()),
            ..Button::default()
        }));
        let n_keys = keys.len();
        let span = |edit: &str, value: Ratio<i64>, tip: &str| {
            Element::Dialer(Dialer {
                on: Some(Edit(edit.into())),
                value: Some(to_f64(value)),
                step: Some(1.0 / 16.0),
                tip: Some(tip.into()),
                ..Dialer::default()
            })
        };
        let bound = |edit: &str, value: Option<f32>, label: &str| {
            Element::Bound(Bound {
                on: Some(Edit(edit.into())),
                value: value.map(f64::from),
                label: Some(label.into()),
                ..Bound::default()
            })
        };
        let toggle = |field: &str, label: &str, tip: &str| {
            Element::Toggle(gantz_ui::Toggle {
                field: Some(Field(field.into())),
                label: Some(label.into()),
                pill: true,
                tip: Some(tip.into()),
                ..gantz_ui::Toggle::default()
            })
        };
        vec![
            row(
                "state",
                Element::Label(Label {
                    text: "0 events".into(),
                    bind: Some(BindPath(vec![])),
                    at: Some(At(vec![SUMMARY.into()])),
                    ..Label::default()
                }),
                1,
            ),
            row(
                "span",
                hrow(vec![
                    span("start", self.start, "start, in cycles"),
                    span("end", self.end, "end, in cycles"),
                ]),
                1,
            ),
            row(
                "res",
                hrow(vec![Element::Choice(res_mode), Element::Dialer(res)]),
                1,
            ),
            row("values", Element::Choice(layout), 1),
            row("keys", vcol(keys), n_keys),
            row(
                "margin",
                Element::Toggle(gantz_ui::Toggle {
                    field: Some(Field("margin".into())),
                    tip: Some(
                        "inset the data within the node frame's margin (rounded corners)".into(),
                    ),
                    ..gantz_ui::Toggle::default()
                }),
                1,
            ),
            row(
                "colour",
                Element::Color(Color {
                    on: Some(Edit("color".into())),
                    value: self.look.color.map(Rgba),
                    theme: true,
                    tip: Some("the line/bar colour".into()),
                    ..Color::default()
                }),
                1,
            ),
            row(
                "range",
                hrow(vec![
                    bound("y-min", self.look.y_min, "minimum"),
                    bound("y-max", self.look.y_max, "maximum"),
                ]),
                1,
            ),
            row(
                "display",
                hrow(vec![
                    toggle("show_grid", "grid", "draw the background grid"),
                    toggle("show_axes", "axes", "draw the axes"),
                    toggle(
                        "interactive",
                        "interact",
                        "show a crosshair and value readout on hover",
                    ),
                ]),
                1,
            ),
        ]
    }
}

impl Leaf {
    fn num(&self) -> Option<f64> {
        match self {
            Leaf::Num(n) => Some(*n),
            _ => None,
        }
    }

    fn text(&self) -> String {
        match self {
            Leaf::Num(n) => format!("{n}"),
            Leaf::Label(s) | Leaf::Opaque(s) => s.clone(),
        }
    }
}

impl ChannelKey {
    fn text(&self) -> Option<String> {
        match self {
            ChannelKey::Whole => None,
            ChannelKey::Index(i) => Some(i.to_string()),
            ChannelKey::Key(k) => Some(k.clone()),
        }
    }
}

impl Default for Look {
    fn default() -> Self {
        Look {
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

impl Default for Pplot {
    fn default() -> Self {
        Pplot {
            start: Ratio::from_integer(0),
            end: Ratio::from_integer(1),
            res: Res::Fixed(Res::DEFAULT_FIXED),
            layout: ValueLayout::default(),
            key_colors: vec![],
            look: Look::default(),
        }
    }
}

impl NodeDef for Pplot {
    const TAG: &'static str = "Pplot";

    fn meta(&self) -> Meta {
        Meta {
            name: KEYWORD.into(),
            description: "Plot a pattern's events over a span of cycles".into(),
            inputs: 2,
            outputs: 1,
            stateful: true,
            requires: vec!["gantz/pattern".into(), MODULE.name.into()],
            ..Meta::default()
        }
    }

    /// Queries the pattern at the first input over the span at the second,
    /// else over the span of the fields, and stores its figures. Passes the
    /// pattern on.
    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        let Some(Some(p)) = ctx.inputs.first() else {
            return Ok("(begin state)".to_string());
        };
        let default = format!("(pat/span {} {})", self.start, self.end);
        let span = match ctx.inputs.get(1) {
            Some(Some(span)) => format!("(pat/as-span {span} {default})"),
            _ => default,
        };
        let res = self.res.slices(self.look.width);
        Ok(format!(
            "(begin (set! state (pat/plot-figures (pat/plot-data {p} {span} {res}) {})) {p})",
            self.conf().expr(),
        ))
    }

    /// A canvas of the figures in the state, which fills a body that
    /// resizes.
    fn view(&self) -> View {
        let canvas = Element::Canvas(gantz_ui::Canvas {
            bind: Some(BindPath(vec![])),
            at: Some(At(vec![FIGURES.into()])),
            grid: self.look.show_grid,
            axes: self.look.show_axes,
            interactive: self.look.interactive,
            ..gantz_ui::Canvas::default()
        });
        View {
            inputs: vec![
                SocketDoc::new("pattern", "the pattern to plot"),
                SocketDoc::new(
                    "span or number",
                    "the cycles to plot. a number n plots 0 to n. unconnected uses the \
                     inspector span",
                ),
            ],
            outputs: vec![SocketDoc::new("pattern", "the input pattern, unchanged")],
            body: Some(canvas),
            resize: Some(Resize {
                width: "width".into(),
                height: "height".into(),
                margin: self.look.margin,
                default: Look::DEFAULT_SIZE,
            }),
            rows: self.rows(),
            hide_state: true,
            fill: true,
            ..View::default()
        }
    }

    fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
        let mut p = self.clone();
        p.apply(edit, value).then_some(p)
    }

    fn init_state(&self) -> Option<Datum> {
        Some(Datum::Seq(vec![]))
    }

    /// Lays the data in the state out again with the look of the node.
    fn sync_state(&self, state: &Datum) -> Option<Datum> {
        let data = state.get(DATA)?;
        let plot = plot_data(data).unwrap_or_default();
        let conf = self.conf();
        let mut next = Datum::Map(vec![
            (FIGURES.into(), figures(&plot, &conf)),
            (SUMMARY.into(), Datum::Str(summary(&plot))),
            (DATA.into(), data.clone()),
        ]);
        next.canonicalize();
        (next != *state).then_some(next)
    }
}

/// The `#%gantz/pattern-plot` builtin module that [`MODULE`] provides.
pub fn builtin() -> BuiltInModule {
    let mut module = BuiltInModule::new("#%gantz/pattern-plot");
    module.register_fn("pat/plot-figures", plot_figures);
    module
}

/// Reads a `(pat.plot <keyword>...)` form, such as
/// `(pat.plot #:end 2 #:fit 2 #:expand #:width 240)`.
pub(crate) fn read_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut p = Pplot::default();
    if let Some(start) = ratio_keyword(&args, "start")? {
        p.start = start;
    }
    if let Some(end) = ratio_keyword(&args, "end")? {
        p.end = end;
    }
    if let Some(n) = args.keyword_int("fixed")? {
        p.res = Res::Fixed(n.clamp(1, i64::from(Res::MAX)) as u16);
    }
    if let Some(pts) = args.keyword_f64("fit")? {
        p.res = Res::Fit(pts as f32);
    }
    if args.has_flag("expand") {
        p.layout = ValueLayout::Expand;
    }
    if let Some(w) = args.keyword_int("width")? {
        p.look.width = w.clamp(0, i64::from(u16::MAX)) as u16;
    }
    if let Some(h) = args.keyword_int("height")? {
        p.look.height = h.clamp(0, i64::from(u16::MAX)) as u16;
    }
    tagged(&p).ok_or_else(|| FormatError::malformed("pat.plot does not encode"))
}

/// The default plot as a node datum.
pub(crate) fn read_bare() -> Option<Datum> {
    tagged(&Pplot::default())
}

/// Writes a plot as a bare `pat.plot` or a keyword form. `None` when a
/// setting has no keyword, so the generic node form applies.
pub(crate) fn write_spec(node: &Datum) -> Option<String> {
    let p: Pplot = gantz_format::from_datum(node.clone()).ok()?;
    let d = Pplot::default();
    // The plot as the keywords can express it. Unless it equals the plot
    // itself, some setting would be lost.
    let expressed = Pplot {
        key_colors: vec![],
        look: Look {
            width: p.look.width,
            height: p.look.height,
            ..Look::default()
        },
        ..p.clone()
    };
    if expressed != p {
        return None;
    }
    let mut kws = vec![];
    if p.start != d.start {
        kws.push(format!("#:start {}", p.start));
    }
    if p.end != d.end {
        kws.push(format!("#:end {}", p.end));
    }
    match p.res {
        Res::Fixed(Res::DEFAULT_FIXED) => (),
        Res::Fixed(n) => kws.push(format!("#:fixed {n}")),
        Res::Fit(pts) => kws.push(format!("#:fit {pts}")),
    }
    if p.layout == ValueLayout::Expand {
        kws.push("#:expand".to_string());
    }
    if p.look.width != d.look.width {
        kws.push(format!("#:width {}", p.look.width));
    }
    if p.look.height != d.look.height {
        kws.push(format!("#:height {}", p.look.height));
    }
    Some(match kws.is_empty() {
        true => KEYWORD.to_string(),
        false => format!("({KEYWORD} {})", kws.join(" ")),
    })
}

/// A plot as a node datum, tagged with its tag.
fn tagged(p: &Pplot) -> Option<Datum> {
    match gantz_nodedef::encode(p).ok()? {
        Datum::Map(fields) => Some(Datum::tagged(Pplot::TAG, fields)),
        _ => None,
    }
}

/// Reads an exact rational keyword value, for example `2` or `1/3`.
fn ratio_keyword(args: &SugarArgs<'_>, key: &str) -> Result<Option<Ratio<i64>>, FormatError> {
    args.keyword_verbatim(key)?
        .map(|src| {
            crate::mini::parse_number(src)
                .ok_or_else(|| FormatError::malformed(format!("#:{key} requires a rational")))
        })
        .transpose()
}

/// Lays the output of `pat/plot-data` out as the panels of a canvas. Gives a
/// map of the `figures`, the `summary` of the events and the `data` itself.
/// A value in the data with no data form, such as a function, plots as an
/// opaque span. Takes the data and a conf of the layout, the colour or `#f` for
/// the theme's, a list of `(key colour)` pairs, and the fixed bounds of the
/// value axis or `#f`.
fn plot_figures(data: SteelVal, conf: SteelVal) -> Result<SteelVal, SteelErr> {
    let conf = datum::of_steel(&conf).ok();
    let conf = conf.as_ref().and_then(read_conf).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::Generic,
            "pat/plot-figures: a malformed conf".to_string(),
        )
    })?;
    let plot = plot_data(&datum::of_steel_lossy(&data)).unwrap_or_default();
    let map = gantz_core::steel::HashMap::new()
        .update(key(FIGURES), datum::to_steel(&figures(&plot, &conf)))
        .update(key(SUMMARY), SteelVal::StringV(summary(&plot).into()))
        .update(key(DATA), data);
    Ok(SteelVal::HashMapV(Gc::new(map).into()))
}

/// The Steel key of an entry of the state of the node, a symbol as
/// `datum::to_steel` writes it.
fn key(name: &str) -> SteelVal {
    SteelVal::SymbolV(name.into())
}

/// The summary of the events of `plot`.
fn summary(plot: &PlotData) -> String {
    let n_events: usize = plot.channels.values().map(|c| c.segments.len()).sum();
    let mut summary = format!("{n_events} events");
    if plot.channels.len() > 1 {
        summary.push_str(&format!(" · {} channels", plot.channels.len()));
    }
    summary
}

/// The panels of `plot` with `conf`, as a list of figure forms.
fn figures(plot: &PlotData, conf: &Conf) -> Datum {
    let chans: Vec<(&ChannelKey, &Channel)> = plot.channels.iter().collect();
    let panels = match conf.layout {
        ValueLayout::Stack => vec![panel(plot.span, &chans, false, conf)],
        ValueLayout::Expand => chans
            .iter()
            .map(|chan| panel(plot.span, std::slice::from_ref(chan), true, conf))
            .collect(),
    };
    Datum::Seq(panels)
}

/// The conf that [`Conf::expr`] writes.
fn read_conf(conf: &Datum) -> Option<Conf> {
    let [layout, color, keys, y_min, y_max] = list_n(conf)?;
    let layout = match text(layout)? {
        "stack" => ValueLayout::Stack,
        "expand" => ValueLayout::Expand,
        _ => return None,
    };
    let key_colors = list(keys)?
        .iter()
        .filter_map(|kc| {
            let [k, c] = list_n(kc)?;
            Some((text(k)?.to_string(), text(c)?.to_string()))
        })
        .collect();
    Some(Conf {
        layout,
        color: text(color).map(str::to_string),
        key_colors,
        y_min: num(y_min),
        y_max: num(y_max),
    })
}

/// One panel of `chans` over `span`. With `titled`, the panel shows the key
/// of its channel.
fn panel(span: [f64; 2], chans: &[(&ChannelKey, &Channel)], titled: bool, conf: &Conf) -> Datum {
    let color_of = |key: &ChannelKey| -> Option<&str> {
        let text = key.text();
        let keyed = conf
            .key_colors
            .iter()
            .find(|(k, _)| Some(k) == text.as_ref());
        keyed.map(|(_, c)| c.as_str()).or(conf.color.as_deref())
    };
    // Place each channel's non-numeric events in lanes below those of the
    // prior channels.
    let mut boxes: Vec<(&ChannelKey, &Seg, usize)> = vec![];
    let mut n_lanes = 0;
    for (key, chan) in chans {
        let segs: Vec<&Seg> = chan
            .segments
            .iter()
            .filter(|s| s.leaf.num().is_none())
            .collect();
        let spans: Vec<[f64; 2]> = segs.iter().map(|s| [s.start, s.end]).collect();
        let (lanes, n) = pack_lanes(&spans);
        boxes.extend(
            segs.into_iter()
                .zip(lanes)
                .map(|(s, l)| (*key, s, n_lanes + l)),
        );
        n_lanes += n;
    }
    let numeric = chans
        .iter()
        .any(|(_, c)| !c.points.is_empty() || c.segments.iter().any(|s| s.leaf.num().is_some()));
    let title = titled
        .then(|| chans.first().and_then(|(k, _)| k.text()))
        .flatten();
    let mut attrs = vec![attr(
        "x",
        [
            Datum::F64(span[0]),
            Datum::F64(span[1].max(span[0] + f64::EPSILON)),
        ],
    )];
    // With no numeric data the value axis spans the lanes. Else it fits the
    // data, unless a bound fixes an edge.
    match numeric {
        false => attrs.push(attr("y", [Datum::F64(0.0), Datum::F64(1.0)])),
        true => {
            attrs.extend(conf.y_min.map(|lo| attr("y-min", [Datum::F64(lo)])));
            attrs.extend(conf.y_max.map(|hi| attr("y-max", [Datum::F64(hi)])));
        }
    }
    attrs.push(attr("pad", [Datum::F64(EDGE_PAD)]));
    attrs.extend(title.map(|title| attr("title", [Datum::Str(title.into())])));
    let mut figs = vec![];
    let lane_h = 1.0 / n_lanes.max(1) as f64;
    for (key, seg, lane) in &boxes {
        let top = 1.0 - lane_h * *lane as f64;
        let mut rect_attrs = vec![attr("frac", [Datum::Bool(true)])];
        rect_attrs.extend(color_attr(color_of(key)));
        if let Leaf::Label(text) = &seg.leaf {
            rect_attrs.push(attr("label", [Datum::Str(text.clone())]));
        }
        rect_attrs.push(attr("tip", [Datum::Str(hover_text(key, &seg.leaf))]));
        let nums = [
            seg.start,
            top - lane_h * (1.0 - LANE_GAP),
            seg.end,
            top - lane_h * LANE_GAP,
        ];
        figs.push(form("rect", rect_attrs, nums.map(Datum::F64)));
    }
    for (key, chan) in chans {
        let color = color_of(key);
        for seg in &chan.segments {
            let Some(y) = seg.leaf.num() else {
                continue;
            };
            let tip = hover_text(key, &seg.leaf);
            figs.push(figure("seg", color, &tip, &[seg.start, y, seg.end, y]));
            if seg.onset {
                figs.push(figure("dot", color, &tip, &[seg.start, y]));
            }
        }
        if !chan.points.is_empty() {
            let points = chan
                .points
                .iter()
                .map(|[x, y]| Datum::Seq(vec![Datum::F64(*x), Datum::F64(*y)]));
            figs.push(form(
                "line",
                color_attr(color).into_iter().collect(),
                points,
            ));
        }
    }
    form("panel", attrs, figs)
}

/// A row of elements.
fn hrow(children: Vec<Element>) -> Element {
    Element::Row(gantz_ui::Row {
        gap: None,
        align: None,
        key: None,
        children,
    })
}

/// A column of elements.
fn vcol(children: Vec<Element>) -> Element {
    Element::Col(gantz_ui::Col {
        gap: None,
        align: None,
        key: None,
        children,
    })
}

/// The number that a control sent.
fn number(value: &Datum) -> Option<f64> {
    match value {
        Datum::F64(f) => Some(*f),
        Datum::I64(i) => Some(*i as f64),
        _ => None,
    }
}

fn to_f64(r: Ratio<i64>) -> f64 {
    *r.numer() as f64 / *r.denom() as f64
}

/// A figure form of numbers, with its colour and tip.
fn figure(tag: &str, color: Option<&str>, tip: &str, nums: &[f64]) -> Datum {
    let mut attrs: Vec<Datum> = color_attr(color).into_iter().collect();
    attrs.push(attr("tip", [Datum::Str(tip.into())]));
    form(tag, attrs, nums.iter().map(|n| Datum::F64(*n)))
}

/// The form `(tag (@ attrs...) items...)`.
fn form(tag: &str, attrs: Vec<Datum>, items: impl IntoIterator<Item = Datum>) -> Datum {
    let attrs = std::iter::once(Datum::Sym("@".into())).chain(attrs);
    let head = [Datum::Sym(tag.into()), Datum::Seq(attrs.collect())];
    Datum::Seq(head.into_iter().chain(items).collect())
}

/// The attribute `(name values...)`.
fn attr(name: &str, values: impl IntoIterator<Item = Datum>) -> Datum {
    let head = std::iter::once(Datum::Sym(name.into()));
    Datum::Seq(head.chain(values).collect())
}

fn color_attr(color: Option<&str>) -> Option<Datum> {
    color.map(|color| attr("color", [Datum::Str(color.into())]))
}

/// Decode the output of `pat/plot-data`. `None` unless the value is the
/// expected `(start end segments points)` list. Malformed entries are
/// skipped.
fn plot_data(val: &Datum) -> Option<PlotData> {
    let [start, end, segments, points] = list_n(val)?;
    let span = [num(start)?, num(end)?];
    let mut channels: BTreeMap<ChannelKey, Channel> = BTreeMap::new();
    for seg in list(segments)? {
        let Some([start, end, value, onset]) = list_n(seg) else {
            continue;
        };
        let (Some(start), Some(end)) = (num(start), num(end)) else {
            continue;
        };
        let onset = *onset == Datum::Bool(true);
        for (key, leaf) in channels_of(value) {
            let seg = Seg {
                start,
                end,
                leaf,
                onset,
            };
            channels.entry(key).or_default().segments.push(seg);
        }
    }
    for pt in list(points)? {
        let Some([x, value]) = list_n(pt) else {
            continue;
        };
        let Some(x) = num(x) else {
            continue;
        };
        // A sample has no width to draw as a span, so only numbers plot.
        for (key, leaf) in channels_of(value) {
            if let Some(y) = leaf.num() {
                channels.entry(key).or_default().points.push([x, y]);
            }
        }
    }
    Some(PlotData { span, channels })
}

/// Split an event value into its channels and their leaves. A map gives one
/// channel per entry, and a list or vector one per element. Anything else is
/// one whole leaf.
fn channels_of(val: &Datum) -> Vec<(ChannelKey, Leaf)> {
    match val {
        Datum::Map(entries) if val.as_opaque().is_none() => entries
            .iter()
            .map(|(k, v)| (ChannelKey::Key(k.clone()), leaf(v)))
            .collect(),
        Datum::Seq(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (ChannelKey::Index(i), leaf(v)))
            .collect(),
        _ => vec![(ChannelKey::Whole, leaf(val))],
    }
}

/// Classify a single value. Containers are opaque here, so only the top level
/// of an event value splits into channels. A non-finite number is opaque, as
/// it has no place on the value axis.
fn leaf(val: &Datum) -> Leaf {
    match val {
        Datum::Bool(b) => Leaf::Num(if *b { 1.0 } else { 0.0 }),
        Datum::Str(s) | Datum::Sym(s) => Leaf::Label(s.clone()),
        Datum::Char(c) => Leaf::Label(c.to_string()),
        _ => match num(val) {
            Some(n) if n.is_finite() => Leaf::Num(n),
            _ => Leaf::Opaque(display(val)),
        },
    }
}

/// The text of a string or a symbol.
fn text(val: &Datum) -> Option<&str> {
    match val {
        Datum::Str(s) | Datum::Sym(s) => Some(s),
        _ => None,
    }
}

/// Any real number as the nearest `f64`.
fn num(val: &Datum) -> Option<f64> {
    val.as_f64()
}

/// The display text of a value, as Steel writes it. A hash map writes its
/// entries on one line.
fn display(val: &Datum) -> String {
    let join = |items: &[Datum]| {
        let items: Vec<String> = items.iter().map(display).collect();
        items.join(" ")
    };
    match val {
        Datum::Seq(items) => format!("({})", join(items)),
        Datum::Map(entries) => match val.as_opaque() {
            Some(what) => format!("#<{what}>"),
            None => {
                let entries: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", display(v)))
                    .collect();
                format!("#<hashmap {{{}}}>", entries.join(", "))
            }
        },
        Datum::Bool(true) => "#true".to_string(),
        Datum::Bool(false) => "#false".to_string(),
        Datum::Null => "#<void>".to_string(),
        Datum::I64(n) => n.to_string(),
        Datum::U64(n) => n.to_string(),
        Datum::F64(x) => format!("{x:?}"),
        Datum::Ratio(r) => format!("{}/{}", r.numer(), r.denom()),
        Datum::Char(c) => format!("#\\{c}"),
        Datum::Str(s) => format!("{s:?}"),
        Datum::Sym(s) => s.clone(),
        Datum::Bytes(b) => {
            let bytes: Vec<String> = b.iter().map(u8::to_string).collect();
            format!("#u8({})", bytes.join(" "))
        }
    }
}

/// The elements of a list value.
fn list(val: &Datum) -> Option<&[Datum]> {
    match val {
        Datum::Seq(items) => Some(items),
        _ => None,
    }
}

/// The elements of a list value of exactly `N` elements.
fn list_n<const N: usize>(val: &Datum) -> Option<&[Datum; N]> {
    list(val)?.try_into().ok()
}

/// The hover text for a value, prefixed by its key when it has one.
fn hover_text(key: &ChannelKey, leaf: &Leaf) -> String {
    match key.text() {
        Some(k) => format!("{k}: {}", leaf.text()),
        None => leaf.text(),
    }
}

/// Greedily pack `spans` into lanes, so spans sharing a lane never overlap.
/// Spans take the first lane free at their start, in start order. Returns
/// each span's lane and the lane count.
fn pack_lanes(spans: &[[f64; 2]]) -> (Vec<usize>, usize) {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|&a, &b| spans[a][0].total_cmp(&spans[b][0]));
    let mut lane_ends: Vec<f64> = vec![];
    let mut lanes = vec![0; spans.len()];
    for i in order {
        let [start, end] = spans[i];
        let lane = match lane_ends.iter().position(|&e| e <= start) {
            Some(lane) => lane,
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
        lane_ends[lane] = end;
        lanes[i] = lane;
    }
    (lanes, lane_ends.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(0.0 1.0 segments ())` with one segment per `(start end value)`, each
    /// an onset. A value that parses as a number is a number, else a symbol.
    fn data(segs: &[(f64, f64, &str)]) -> Datum {
        let segs = segs.iter().map(|(start, end, v)| {
            let value = match v.parse::<i64>() {
                Ok(n) => Datum::I64(n),
                Err(_) => Datum::Sym(v.to_string()),
            };
            let (start, end) = (Datum::F64(*start), Datum::F64(*end));
            Datum::Seq(vec![start, end, value, Datum::Bool(true)])
        });
        Datum::Seq(vec![
            Datum::F64(0.0),
            Datum::F64(1.0),
            Datum::Seq(segs.collect()),
            Datum::Seq(vec![]),
        ])
    }

    /// The tags of the figures of each panel.
    fn tags(figures: &Datum) -> Vec<Vec<String>> {
        let panels = list(figures).unwrap();
        panels
            .iter()
            .map(|panel| {
                list(panel).unwrap()[2..]
                    .iter()
                    .filter_map(|f| text(&list(f)?[0]).map(str::to_string))
                    .collect()
            })
            .collect()
    }

    fn stack() -> Conf {
        Conf::default()
    }

    #[test]
    fn symbols_draw_as_labelled_boxes_in_lanes() {
        let plot = plot_data(&data(&[(0.0, 0.5, "bd"), (0.5, 1.0, "sn")])).unwrap();
        assert_eq!(tags(&figures(&plot, &stack())), [["rect", "rect"]]);
    }

    #[test]
    fn numbers_draw_as_segments_with_onset_dots() {
        let plot = plot_data(&data(&[(0.0, 0.5, "1"), (0.5, 1.0, "2")])).unwrap();
        let tags = tags(&figures(&plot, &stack()));
        assert_eq!(tags, [["seg", "dot", "seg", "dot"]]);
    }

    #[test]
    fn overlapping_spans_take_their_own_lanes() {
        assert_eq!(pack_lanes(&[]), (vec![], 0));
        assert_eq!(pack_lanes(&[[0.0, 0.5], [0.5, 1.0]]), (vec![0, 0], 1));
        assert_eq!(
            pack_lanes(&[[0.0, 1.0], [0.0, 0.5], [0.5, 1.0]]),
            (vec![0, 1, 1], 2)
        );
    }

    #[test]
    fn values_display_as_steel_writes_them() {
        let v = Datum::Seq(vec![
            Datum::I64(1),
            Datum::Seq(vec![Datum::Sym("a".into())]),
            Datum::Str("s".into()),
        ]);
        assert_eq!(display(&v), "(1 (a) \"s\")");
        let v = Datum::Seq(vec![
            Datum::F64(1.0),
            Datum::Char('a'),
            Datum::Bool(true),
            Datum::ratio(1, 3).unwrap(),
        ]);
        assert_eq!(display(&v), "(1.0 #\\a #true 1/3)");
    }

    /// The conf that the expr quotes reads back as the conf.
    #[test]
    fn the_conf_reads_back() {
        let mut p = Pplot::default();
        assert!(p.apply("key-add", &Datum::Null));
        assert!(p.apply("key-name-0", &Datum::Str("s".into())));
        assert!(p.apply("y-min", &Datum::F64(-1.0)));
        let conf = p.conf();
        let mut vm = gantz_core::steel::steel_vm::engine::Engine::new();
        let quoted = vm.run(conf.expr()).unwrap();
        let read = read_conf(&datum::of_steel(quoted.last().unwrap()).unwrap());
        assert_eq!(read, Some(conf));
    }

    /// A fitted resolution samples once per this many points of the body.
    #[test]
    fn fit_res_follows_the_width() {
        assert_eq!(Res::Fixed(64).slices(500), 64);
        assert_eq!(Res::Fit(2.0).slices(240), 120);
        assert_eq!(Res::Fit(0.0).slices(10), 20);
        assert_eq!(Res::Fixed(0).slices(10), 1);
    }

    /// The span edits snap to the grid and keep the end after the start.
    #[test]
    fn span_edits_snap_and_order() {
        let mut p = Pplot::default();
        assert!(p.apply("start", &Datum::F64(2.0)));
        assert_eq!(p.start, Ratio::from_integer(2));
        assert_eq!(p.end, Ratio::from_integer(2) + SPAN_STEP);
        assert!(!p.apply("end", &Datum::F64(0.5)));
        assert!(p.apply("end", &Datum::F64(3.0)));
        assert_eq!(p.end, Ratio::from_integer(3));
    }

    /// Key colours add, rename, recolour and remove, and colour their
    /// channels.
    #[test]
    fn key_colours_edit_and_apply() {
        let mut p = Pplot::default();
        assert!(p.apply("key-add", &Datum::Null));
        assert!(p.apply("key-name-0", &Datum::Str("s".into())));
        assert!(p.apply("key-color-0", &Datum::Str("#ff0000ff".into())));
        assert_eq!(
            p.key_colors,
            [KeyColor {
                key: "s".into(),
                color: [255, 0, 0, 255]
            }]
        );
        let conf = p.conf().expr();
        assert!(conf.contains("(\"s\" \"#ff0000\")"), "{conf}");
        assert!(!p.apply("key-name-3", &Datum::Str("x".into())));
        assert!(p.apply("key-remove-0", &Datum::Null));
        assert!(p.key_colors.is_empty());
    }

    /// Keyword forms round trip, including exact rationals. A default plot
    /// is bare.
    #[test]
    fn keywords_round_trip() {
        let bare = read_bare().expect("bare");
        assert_eq!(write_spec(&bare).as_deref(), Some("pat.plot"));
        let mut p = Pplot::default();
        p.start = Ratio::new(1, 3);
        p.res = Res::Fit(2.5);
        p.layout = ValueLayout::Expand;
        p.look.width = 240;
        let datum = tagged(&p).unwrap();
        assert_eq!(
            write_spec(&datum).as_deref(),
            Some("(pat.plot #:start 1/3 #:fit 2.5 #:expand #:width 240)")
        );
        p.key_colors.push(KeyColor {
            key: "s".into(),
            color: [0; 4],
        });
        assert_eq!(write_spec(&tagged(&p).unwrap()), None);
    }

    /// The state keeps the plot data, so an edit of the look lays the data
    /// out again before the next evaluation.
    #[test]
    fn a_look_edit_lays_the_data_out_again() {
        let data = data(&[(0.0, 0.5, "bd"), (0.5, 1.0, "sn")]);
        let state = Datum::Map(vec![(DATA.into(), data.clone())]);
        let mut p = Pplot::default();
        let laid = p.sync_state(&state).expect("the figures of the data");
        assert_eq!(laid.get(DATA), Some(&data), "the data stays");
        assert!(laid.get(FIGURES).is_some());
        assert_eq!(p.sync_state(&laid), None, "the same look keeps the state");
        assert!(p.apply("color", &Datum::Str("#ff0000ff".into())));
        let recoloured = p.sync_state(&laid).expect("a new look lays out again");
        assert_ne!(recoloured.get(FIGURES), laid.get(FIGURES));
        assert_eq!(
            p.sync_state(&Datum::Seq(vec![])),
            None,
            "no data, no figures"
        );
    }

    /// The native fn gives the figures and the summary of the data with the
    /// data itself, as the state of the node holds them.
    #[test]
    fn plot_figures_gives_the_state_of_the_node() {
        let data = data(&[(0.0, 0.5, "bd")]);
        let conf = Pplot::default().conf();
        let mut vm = gantz_core::steel::steel_vm::engine::Engine::new();
        let quoted = vm.run(conf.expr()).unwrap();
        let state = plot_figures(datum::to_steel(&data), quoted[0].clone()).unwrap();
        let state = datum::of_steel(&state).unwrap();
        assert_eq!(state.get(SUMMARY), Some(&Datum::Str("1 events".into())));
        assert_eq!(state.get(DATA), Some(&data));
        let plot = plot_data(&data).unwrap();
        assert_eq!(state.get(FIGURES), Some(&figures(&plot, &conf)));
    }
}
