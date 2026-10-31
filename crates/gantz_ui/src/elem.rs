//! The typed UI tree model.
//!
//! [`Element`] is the canonical, closed representation of the v1 vocabulary.
//! Unknown or malformed forms never fail a decode. They become
//! [`Element::Error`] nodes in place. See [`crate::diag`].
//!
//! The `Default` impl of each element struct is the single source of truth
//! for attribute defaults. The decoder falls back to it and the encoder
//! omits attributes equal to it.

use crate::diag::{ErrorReason, TreePath};
use std::fmt;

/// A node path relative to the graph the tree was defined in.
///
/// Segments match `gantz_core::node::Id`, a plain `usize`. They are stated
/// as `usize` here so the model stays runtime free. See the crate doc's
/// binding model.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct BindPath(pub Vec<usize>);

/// An explicit identity override, `(key <string|int>)`.
///
/// See the crate doc on identity and keys.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Key {
    /// A string key.
    Str(String),
    /// An integer key.
    Int(i64),
}

/// An RGBA colour parsed from `"#rrggbb"` or `"#rrggbbaa"`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Rgba(pub [u8; 4]);

/// Cross axis alignment for `col` and `row`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Align {
    /// Align children to the start of the cross axis.
    Start,
    /// Center children on the cross axis.
    Center,
    /// Align children to the end of the cross axis.
    End,
}

/// The rendering style of a `dialer`. Reserved. The v1 host renders every
/// dialer as a drag value regardless.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DialerStyle {
    /// A linear slider.
    Slider,
    /// A rotary knob.
    Knob,
}

/// A theme colour for text, as a host's theme gives it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Tone {
    /// Text of less weight, such as a note.
    Weak,
    /// Text of more weight, such as a heading.
    Strong,
    /// A warning.
    Warn,
    /// An error.
    Error,
}

/// What a `plot` displays.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PlotMode {
    /// A rolling history buffer.
    Scope,
    /// The bound value itself as a signal frame.
    Signal,
}

/// How a `plot` draws its samples.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PlotStyle {
    /// One bar per sample.
    Bars,
    /// A connected line.
    Line,
}

/// One node of the decoded UI tree.
///
/// The model is closed. Unknown tags decode to [`Element::Error`] so they
/// stay visible, never silently blank.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    /// A vertical stack, `(col ...)`.
    Col(Col),
    /// A horizontal stack, `(row ...)`.
    Row(Row),
    /// A row-major grid, `(grid (@ (cols n)) ...)`.
    Grid(Grid),
    /// A labelled group box, `(frame ...)`.
    Frame(Frame),
    /// A separator line, `(sep)`.
    Sep(Sep),
    /// Empty space, `(space n)`.
    Space(Space),
    /// A binding path prefix for a subtree, `(scope id ...)`.
    Scope(Scope),
    /// A numeric control, `(dialer)`.
    Dialer(Dialer),
    /// A boolean control, `(toggle)`.
    Toggle(Toggle),
    /// An optional number, `(bound)`.
    Bound(Bound),
    /// One of a set of options, `(choice)`.
    Choice(Choice),
    /// A colour, `(color)`.
    Color(Color),
    /// A line of text, `(text-input)`.
    TextInput(TextInput),
    /// A trigger control, `(button)`.
    Button(Button),
    /// A button that imports a file into the node, `(file)`.
    File(File),
    /// A grid of bool or number cells, `(matrix)`.
    Matrix(Matrix),
    /// Static text, `(label "text")`.
    Label(Label),
    /// A read-only value readout, `(value)`.
    Value(Value),
    /// A plotted view of bound state, `(plot)`.
    Plot(Plot),
    /// A figure that bound state holds, `(canvas)`. See [`crate::figure`].
    Canvas(Canvas),
    /// An editor of a breakpoint envelope, `(envelope)`. See
    /// [`crate::envelope`].
    Envelope(EnvelopeElem),
    /// A host resolved embed of a child instance's GUI, `(ref-gui id)`.
    RefGui(RefGui),
    /// A subtree that could not be decoded, rendered as an inline error.
    Error(ErrorElem),
}

/// A vertical stack of children.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Col {
    /// Spacing between children, host default when absent.
    pub gap: Option<f32>,
    /// Cross axis alignment, host default when absent.
    pub align: Option<Align>,
    /// Identity override.
    pub key: Option<Key>,
    /// Child elements.
    pub children: Vec<Element>,
}

/// A horizontal stack of children.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// Spacing between children, host default when absent.
    pub gap: Option<f32>,
    /// Cross axis alignment, host default when absent.
    pub align: Option<Align>,
    /// Identity override.
    pub key: Option<Key>,
    /// Child elements.
    pub children: Vec<Element>,
}

/// A grid filled row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// The number of columns, at least 1. Required. Decoding substitutes 1
    /// with a warning when absent.
    pub cols: u32,
    /// Spacing between cells, host default when absent.
    pub gap: Option<f32>,
    /// Identity override.
    pub key: Option<Key>,
    /// Child elements.
    pub children: Vec<Element>,
}

/// A labelled group box.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// The group box title.
    pub title: Option<String>,
    /// Identity override.
    pub key: Option<Key>,
    /// Child elements.
    pub children: Vec<Element>,
}

/// A separator line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sep {
    /// Identity override.
    pub key: Option<Key>,
}

/// Empty space along the parent's main axis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Space {
    /// The amount of space, host default when absent. Positional, as in
    /// `(space 8)`.
    pub amount: Option<f32>,
    /// Identity override.
    pub key: Option<Key>,
}

/// Prefixes the binding paths of a subtree with a node id.
///
/// Codegen inserts it where a child graph's exported GUI crosses into a
/// parent expression. Hosts apply it implicitly when rendering a referenced
/// graph at an instance path.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    /// The node id pushed onto the binding path prefix. Positional, as in
    /// `(scope 3 ...)`.
    pub id: usize,
    /// Identity override.
    pub key: Option<Key>,
    /// Child elements, rendered with the prefixed scope.
    pub children: Vec<Element>,
}

/// A numeric control bound to number state.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialer {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// The keys of the value within the bound state. See [`At`].
    pub at: Option<At>,
    /// The bound field of the node that owns the tree. See [`Field`].
    pub field: Option<Field>,
    /// The edit that the control sends to the node that owns the tree. See
    /// [`Edit`].
    pub on: Option<Edit>,
    /// The value shown by a control that sends an edit.
    pub value: Option<f64>,
    /// Lower bound.
    pub min: Option<f64>,
    /// Upper bound.
    pub max: Option<f64>,
    /// Drag step.
    pub step: Option<f64>,
    /// Displayed decimal places.
    pub precision: Option<u8>,
    /// An inline label.
    pub label: Option<String>,
    /// Text drawn inside the value box, before the value.
    pub prefix: Option<String>,
    /// Text drawn inside the value box, after the value.
    pub suffix: Option<String>,
    /// Whether a `set` also queues a push eval at the bound node.
    pub push: bool,
    /// Rendering style, reserved.
    pub style: Option<DialerStyle>,
    /// Text shown while the pointer hovers the control.
    pub tip: Option<String>,
    /// Identity override.
    pub key: Option<Key>,
}

/// A boolean control bound to bool state.
#[derive(Clone, Debug, PartialEq)]
pub struct Toggle {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// The keys of the value within the bound state. See [`At`].
    pub at: Option<At>,
    /// The bound field of the node that owns the tree. See [`Field`].
    pub field: Option<Field>,
    /// The edit that the control sends to the node that owns the tree. See
    /// [`Edit`].
    pub on: Option<Edit>,
    /// The value shown by a control that sends an edit.
    pub value: bool,
    /// An inline label.
    pub label: Option<String>,
    /// Whether a `set` also queues a push eval at the bound node.
    pub push: bool,
    /// Whether it shows as a pill that lights while on, as a display option
    /// does, rather than as a checkbox. The pill shows the label.
    pub pill: bool,
    /// Text shown while the pointer hovers the control.
    pub tip: Option<String>,
    /// Identity override.
    pub key: Option<Key>,
}

/// An optional number bound to number or `#f` state. A checkbox sets the
/// bound state to the number beside it, or clears it to `#f`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bound {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// The keys of the value within the bound state. See [`At`].
    pub at: Option<At>,
    /// The bound field of the node that owns the tree. See [`Field`].
    pub field: Option<Field>,
    /// The edit that the control sends to the node that owns the tree. See
    /// [`Edit`].
    pub on: Option<Edit>,
    /// The bound shown by a control that sends an edit. `None` shows it
    /// unset.
    pub value: Option<f64>,
    /// The checkbox's hint, such as `minimum`.
    pub label: Option<String>,
    /// Identity override.
    pub key: Option<Key>,
}

/// One of a set of options, bound to state that holds one of them. A pill
/// per option sets the bound state to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// The keys of the value within the bound state. See [`At`].
    pub at: Option<At>,
    /// The bound field of the node that owns the tree. See [`Field`].
    pub field: Option<Field>,
    /// The edit that the control sends to the node that owns the tree. See
    /// [`Edit`].
    pub on: Option<Edit>,
    /// The option shown by a control that sends an edit.
    pub value: Option<String>,
    /// The options, as symbols.
    pub options: Vec<String>,
    /// Text shown while the pointer hovers an option, one per option.
    pub tips: Vec<String>,
    /// Identity override.
    pub key: Option<Key>,
}

/// A line of text bound to string state. An edit commits when the input
/// loses focus, on enter, or once typing pauses, so each keystroke is not a
/// write.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInput {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// The keys of the value within the bound state. See [`At`].
    pub at: Option<At>,
    /// The bound field of the node that owns the tree. See [`Field`].
    pub field: Option<Field>,
    /// The edit that the control sends to the node that owns the tree. See
    /// [`Edit`].
    pub on: Option<Edit>,
    /// The text shown by a control that sends an edit.
    pub value: Option<String>,
    /// Text shown while the input is empty.
    pub hint: Option<String>,
    /// Whether the text shows in a monospace font, as code does.
    pub mono: bool,
    /// Whether a commit to bound state also queues a push eval at the bound
    /// node.
    pub push: bool,
    /// Text shown while the pointer hovers the control.
    pub tip: Option<String>,
    /// Identity override.
    pub key: Option<Key>,
}

/// A path of keys into the bound state, `(at freq value)`.
///
/// A control with a `bind` and an `at` reads the value at these keys of the
/// nested hash maps of the bound state. A write replaces only that value, so
/// the other entries of each map stay. A key matches a symbol or a string key
/// of the same name. A write creates a missing map on the way.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct At(pub Vec<String>);

/// The name of a top-level field of the node that owns a tree.
///
/// A node may give a tree as its own view. A control bound to one of its
/// fields reads the field and, on an edit, sets it in the node's stored
/// form, which changes the node. The host checks the edit by decoding the
/// node again. Elsewhere, such as in a graph's GUI, a field-bound control is
/// inert.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Field(pub String);

/// The name of an edit that a control sends to the node that owns the tree.
///
/// A node may give a tree as its own view. A control with an `on` edit shows
/// the value of its `value` attribute. On user input it sends the edit, with
/// its new value, to the node. The node applies the edit, so it can check
/// and adjust the value, or change several fields at once. A button sends no
/// value. Elsewhere, such as in a graph's GUI, a control with an edit is
/// inert.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Edit(pub String);

/// A colour bound to `"#rrggbb"` or `#f` state. `#f` follows the theme, and
/// a `theme` button clears an edited colour back to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Color {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// The keys of the value within the bound state. See [`At`].
    pub at: Option<At>,
    /// The bound field of the node that owns the tree. See [`Field`].
    pub field: Option<Field>,
    /// The edit that the control sends to the node that owns the tree. See
    /// [`Edit`].
    pub on: Option<Edit>,
    /// The colour shown by a control that sends an edit.
    pub value: Option<Rgba>,
    /// Whether the `theme` button shows. Without it, the colour stays set.
    pub theme: bool,
    /// Text shown while the pointer hovers the control.
    pub tip: Option<String>,
    /// Identity override.
    pub key: Option<Key>,
}

/// A trigger control. It holds no state. A press queues a push eval at the
/// bound node. Bang semantics stay in the node's expression.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Button {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// The edit that a press sends to the node that owns the tree. See
    /// [`Edit`].
    pub on: Option<Edit>,
    /// An inline label.
    pub label: Option<String>,
    /// Text shown while the pointer hovers the control.
    pub tip: Option<String>,
    /// Identity override.
    pub key: Option<Key>,
}

/// A button that imports a file into the node that owns the tree.
///
/// A press lets the user pick a file that the host's importer named
/// `import` reads. The host stores the blob that the importer gives, then
/// sends the `on` edit to the node with the blob's address and the
/// importer's data. Elsewhere, such as in a graph's GUI, it is inert.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct File {
    /// The edit that the host sends with the import.
    pub on: Option<Edit>,
    /// The name of the importer that reads the file.
    pub import: Option<String>,
    /// An inline label.
    pub label: Option<String>,
    /// Text shown while the pointer hovers the control.
    pub tip: Option<String>,
    /// Identity override.
    pub key: Option<Key>,
}

/// A grid of bool or number cells bound to list-of-rows state.
///
/// Rows and columns come from the bound state's shape, not from attributes.
/// A dynamic collection is one widget bound to one structured value.
#[derive(Clone, Debug, PartialEq)]
pub struct Matrix {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// The size of one cell, host default when absent. Attribute
    /// `cell-size`.
    pub cell_size: Option<f32>,
    /// Whether a `set` also queues a push eval at the bound node.
    pub push: bool,
    /// Identity override.
    pub key: Option<Key>,
}

/// Static text. The text is positional, as in `(label "text")`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Label {
    /// The text to display.
    pub text: String,
    /// The bound node path. A bound label shows the text that the state
    /// there holds, and `text` while it holds none.
    pub bind: Option<BindPath>,
    /// The keys of the text within the bound state. See [`At`].
    pub at: Option<At>,
    /// Font size, host default when absent.
    pub size: Option<f32>,
    /// Text colour, host default when absent.
    pub color: Option<Rgba>,
    /// A theme colour, used when `color` is absent.
    pub tone: Option<Tone>,
    /// Whether the text shows in a monospace font, as code does.
    pub mono: bool,
    /// Text shown while the pointer hovers the label.
    pub tip: Option<String>,
    /// Identity override.
    pub key: Option<Key>,
}

/// A read-only readout of the bound state's representation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Value {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// Whether long output wraps.
    pub wrap: bool,
    /// Identity override.
    pub key: Option<Key>,
}

/// A plotted view of bound state, a scope buffer or a signal. A list of
/// lists renders as stacked channels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plot {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// What is displayed, host default when absent.
    pub mode: Option<PlotMode>,
    /// How samples draw, host default when absent.
    pub style: Option<PlotStyle>,
    /// Plot colour, theme default when absent.
    pub color: Option<Rgba>,
    /// Whether a grid draws behind the samples.
    pub grid: bool,
    /// Whether axes draw.
    pub axes: bool,
    /// Whether hovering the samples shows a value readout.
    pub interactive: bool,
    /// A fixed lower value axis bound. Attribute `y-min`.
    pub y_min: Option<f32>,
    /// A fixed upper value axis bound. Attribute `y-max`.
    pub y_max: Option<f32>,
    /// Width, host default when absent.
    pub w: Option<f32>,
    /// Height, host default when absent.
    pub h: Option<f32>,
    /// How many of the newest samples of each channel show, all when absent.
    pub capacity: Option<u32>,
    /// Identity override.
    pub key: Option<Key>,
}

/// A drawing of the figures in bound state.
///
/// The bound state is a list of panels, which stack top to bottom. See
/// [`crate::figure`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Canvas {
    /// The bound node path.
    pub bind: Option<BindPath>,
    /// The keys of the figures within the bound state. See [`At`].
    pub at: Option<At>,
    /// Whether a grid draws behind the figures.
    pub grid: bool,
    /// Whether axes draw.
    pub axes: bool,
    /// Whether hovering shows a crosshair and the value beneath it. A tip
    /// shows on hover either way.
    pub interactive: bool,
    /// Width, host default when absent.
    pub w: Option<f32>,
    /// Height, host default when absent.
    pub h: Option<f32>,
    /// Identity override.
    pub key: Option<Key>,
}

/// An editor of a breakpoint envelope. See [`crate::envelope`].
///
/// It shows its `value` envelope. Points drag, segments bend, a double
/// click adds a point, and each point has a menu for its segment's shape,
/// the release point and a delete. Each step of a drag goes to the node
/// that owns the tree as a live edit, and the drag's end as its `on` edit,
/// so a drag is one change of the node. See [`Edit`]. Elsewhere, such as in
/// a graph's GUI, it is inert.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnvelopeElem {
    /// The edit that the editor sends to the node that owns the tree.
    pub on: Option<Edit>,
    /// The envelope to show.
    pub value: Option<crate::envelope::Envelope>,
    /// The time range in seconds, `[min, max]`. The envelope's duration
    /// when absent.
    pub x_range: Option<[f32; 2]>,
    /// The level range, `[min, max]`. `[0, 1]` when absent.
    pub y_range: Option<[f32; 2]>,
    /// Whether a grid draws behind the envelope.
    pub grid: bool,
    /// Whether axes draw.
    pub axes: bool,
    /// Width, the available width when absent.
    pub w: Option<f32>,
    /// Height, the available height when absent.
    pub h: Option<f32>,
    /// Identity override.
    pub key: Option<Key>,
}

/// A host resolved embed of a child instance's GUI.
///
/// The id is the instance's node id in the defining graph. The host resolves
/// and renders it as described in the crate doc on embedding.
#[derive(Clone, Debug, PartialEq)]
pub struct RefGui {
    /// The child instance's node id. Positional, as in `(ref-gui 9)`.
    pub id: usize,
    /// Identity override.
    pub key: Option<Key>,
}

/// A subtree that failed to decode, preserved in place so hosts render an
/// inline error chip while siblings render normally.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorElem {
    /// Where in the tree the failure sits.
    pub path: TreePath,
    /// What went wrong.
    pub reason: ErrorReason,
}

/// Tag names claimed by the vocabulary but not yet implemented. Decoding one
/// yields an [`ErrorReason::ReservedTag`] element.
pub const RESERVED_TAGS: &[&str] = &[
    "tabs", "page", "paint", "image", "dropdown", "xy-pad", "meter",
];

/// The reserved attribute block marker, invalid as a user tag.
pub const ATTRS_MARKER: &str = "@";

impl Element {
    /// The element's tag name. `"error"` for [`Element::Error`].
    pub fn tag(&self) -> &'static str {
        match self {
            Element::Col(_) => "col",
            Element::Row(_) => "row",
            Element::Grid(_) => "grid",
            Element::Frame(_) => "frame",
            Element::Sep(_) => "sep",
            Element::Space(_) => "space",
            Element::Scope(_) => "scope",
            Element::Dialer(_) => "dialer",
            Element::Toggle(_) => "toggle",
            Element::Bound(_) => "bound",
            Element::Choice(_) => "choice",
            Element::Color(_) => "color",
            Element::TextInput(_) => "text-input",
            Element::Button(_) => "button",
            Element::File(_) => "file",
            Element::Matrix(_) => "matrix",
            Element::Label(_) => "label",
            Element::Value(_) => "value",
            Element::Plot(_) => "plot",
            Element::Canvas(_) => "canvas",
            Element::Envelope(_) => "envelope",
            Element::RefGui(_) => "ref-gui",
            Element::Error(_) => "error",
        }
    }

    /// The element's children, empty for leaves.
    pub fn children(&self) -> &[Element] {
        match self {
            Element::Col(e) => &e.children,
            Element::Row(e) => &e.children,
            Element::Grid(e) => &e.children,
            Element::Frame(e) => &e.children,
            Element::Scope(e) => &e.children,
            _ => &[],
        }
    }

    /// The element's explicit identity key, if any.
    pub fn key(&self) -> Option<&Key> {
        match self {
            Element::Col(e) => e.key.as_ref(),
            Element::Row(e) => e.key.as_ref(),
            Element::Grid(e) => e.key.as_ref(),
            Element::Frame(e) => e.key.as_ref(),
            Element::Sep(e) => e.key.as_ref(),
            Element::Space(e) => e.key.as_ref(),
            Element::Scope(e) => e.key.as_ref(),
            Element::Dialer(e) => e.key.as_ref(),
            Element::Toggle(e) => e.key.as_ref(),
            Element::Bound(e) => e.key.as_ref(),
            Element::Choice(e) => e.key.as_ref(),
            Element::Color(e) => e.key.as_ref(),
            Element::TextInput(e) => e.key.as_ref(),
            Element::Button(e) => e.key.as_ref(),
            Element::File(e) => e.key.as_ref(),
            Element::Matrix(e) => e.key.as_ref(),
            Element::Label(e) => e.key.as_ref(),
            Element::Value(e) => e.key.as_ref(),
            Element::Plot(e) => e.key.as_ref(),
            Element::Canvas(e) => e.key.as_ref(),
            Element::Envelope(e) => e.key.as_ref(),
            Element::RefGui(e) => e.key.as_ref(),
            Element::Error(_) => None,
        }
    }
}

impl Align {
    /// The identifier for this alignment in the tree form.
    pub fn name(self) -> &'static str {
        match self {
            Align::Start => "start",
            Align::Center => "center",
            Align::End => "end",
        }
    }

    /// Parse an alignment from its identifier.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "start" => Some(Align::Start),
            "center" => Some(Align::Center),
            "end" => Some(Align::End),
            _ => None,
        }
    }
}

impl DialerStyle {
    /// The identifier for this style in the tree form.
    pub fn name(self) -> &'static str {
        match self {
            DialerStyle::Slider => "slider",
            DialerStyle::Knob => "knob",
        }
    }

    /// Parse a style from its identifier.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "slider" => Some(DialerStyle::Slider),
            "knob" => Some(DialerStyle::Knob),
            _ => None,
        }
    }
}

impl Tone {
    /// The identifier for this tone in the tree form.
    pub fn name(self) -> &'static str {
        match self {
            Tone::Weak => "weak",
            Tone::Strong => "strong",
            Tone::Warn => "warn",
            Tone::Error => "error",
        }
    }

    /// Parse a tone from its identifier.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "weak" => Some(Tone::Weak),
            "strong" => Some(Tone::Strong),
            "warn" => Some(Tone::Warn),
            "error" => Some(Tone::Error),
            _ => None,
        }
    }
}

impl PlotMode {
    /// The identifier for this mode in the tree form.
    pub fn name(self) -> &'static str {
        match self {
            PlotMode::Scope => "scope",
            PlotMode::Signal => "signal",
        }
    }

    /// Parse a mode from its identifier.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "scope" => Some(PlotMode::Scope),
            "signal" => Some(PlotMode::Signal),
            _ => None,
        }
    }
}

impl PlotStyle {
    /// The identifier for this style in the tree form.
    pub fn name(self) -> &'static str {
        match self {
            PlotStyle::Bars => "bars",
            PlotStyle::Line => "line",
        }
    }

    /// Parse a style from its identifier.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "bars" => Some(PlotStyle::Bars),
            "line" => Some(PlotStyle::Line),
            _ => None,
        }
    }
}

impl Rgba {
    /// Parse `"#rrggbb"` or `"#rrggbbaa"`, case insensitive.
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#')?;
        if hex.len() != 6 && hex.len() != 8 {
            return None;
        }
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |ix: usize| u8::from_str_radix(hex.get(ix..ix + 2)?, 16).ok();
        let r = byte(0)?;
        let g = byte(2)?;
        let b = byte(4)?;
        let a = if hex.len() == 8 { byte(6)? } else { 255 };
        Some(Rgba([r, g, b, a]))
    }

    /// The colour as `"#rrggbb"`, or `"#rrggbbaa"` when not opaque. The
    /// inverse of [`Rgba::parse`].
    pub fn hex(&self) -> String {
        let Rgba([r, g, b, a]) = *self;
        match a {
            255 => format!("#{r:02x}{g:02x}{b:02x}"),
            _ => format!("#{r:02x}{g:02x}{b:02x}{a:02x}"),
        }
    }
}

impl Default for Grid {
    fn default() -> Self {
        Grid {
            cols: 1,
            gap: None,
            key: None,
            children: Vec::new(),
        }
    }
}

impl Default for Dialer {
    fn default() -> Self {
        Dialer {
            bind: None,
            at: None,
            field: None,
            on: None,
            value: None,
            min: None,
            max: None,
            step: None,
            precision: None,
            label: None,
            prefix: None,
            suffix: None,
            push: true,
            style: None,
            tip: None,
            key: None,
        }
    }
}

impl Default for TextInput {
    fn default() -> Self {
        TextInput {
            bind: None,
            at: None,
            field: None,
            on: None,
            value: None,
            hint: None,
            mono: false,
            push: true,
            tip: None,
            key: None,
        }
    }
}

impl Default for Toggle {
    fn default() -> Self {
        Toggle {
            bind: None,
            at: None,
            field: None,
            on: None,
            value: false,
            label: None,
            push: true,
            pill: false,
            tip: None,
            key: None,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color {
            bind: None,
            at: None,
            field: None,
            on: None,
            value: None,
            theme: true,
            tip: None,
            key: None,
        }
    }
}

impl Default for Matrix {
    fn default() -> Self {
        Matrix {
            bind: None,
            cell_size: None,
            push: true,
            key: None,
        }
    }
}

impl fmt::Display for Rgba {
    /// Formats lowercase, `#rrggbb` when fully opaque, else `#rrggbbaa`.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Rgba([r, g, b, a]) = *self;
        match a {
            255 => write!(f, "#{r:02x}{g:02x}{b:02x}"),
            _ => write!(f, "#{r:02x}{g:02x}{b:02x}{a:02x}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_parses_rgb_and_rgba() {
        assert_eq!(Rgba::parse("#ff0080"), Some(Rgba([255, 0, 128, 255])));
        assert_eq!(Rgba::parse("#FF0080"), Some(Rgba([255, 0, 128, 255])));
        assert_eq!(Rgba::parse("#ff008040"), Some(Rgba([255, 0, 128, 64])));
    }

    #[test]
    fn rgba_rejects_malformed() {
        assert_eq!(Rgba::parse("ff0080"), None);
        assert_eq!(Rgba::parse("#ff008"), None);
        assert_eq!(Rgba::parse("#gg0080"), None);
        assert_eq!(Rgba::parse("#+f0080"), None);
        assert_eq!(Rgba::parse("#ff00800"), None);
    }

    #[test]
    fn rgba_displays_minimal_form() {
        assert_eq!(Rgba([255, 0, 128, 255]).to_string(), "#ff0080");
        assert_eq!(Rgba([255, 0, 128, 64]).to_string(), "#ff008040");
        assert_eq!(Rgba::parse("#ff008040").unwrap().to_string(), "#ff008040");
    }

    #[test]
    fn enum_names_round_trip() {
        for align in [Align::Start, Align::Center, Align::End] {
            assert_eq!(Align::from_name(align.name()), Some(align));
        }
        for style in [DialerStyle::Slider, DialerStyle::Knob] {
            assert_eq!(DialerStyle::from_name(style.name()), Some(style));
        }
        for mode in [PlotMode::Scope, PlotMode::Signal] {
            assert_eq!(PlotMode::from_name(mode.name()), Some(mode));
        }
        for style in [PlotStyle::Bars, PlotStyle::Line] {
            assert_eq!(PlotStyle::from_name(style.name()), Some(style));
        }
    }

    #[test]
    fn control_defaults_push() {
        assert!(Dialer::default().push);
        assert!(Toggle::default().push);
        assert!(Matrix::default().push);
        assert_eq!(Grid::default().cols, 1);
        assert!(!Value::default().wrap);
        assert!(!Plot::default().grid);
        assert!(!Plot::default().axes);
    }
}
