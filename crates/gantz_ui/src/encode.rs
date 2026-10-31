//! The canonical encoder from the typed tree to a [`Datum`].
//!
//! Canonical form keeps stored GUI values minimal and stable. It mirrors the
//! ext conventions elsewhere in gantz:
//!
//! - Tags and attribute names emit as symbols.
//! - Only attributes that differ from the element's `Default` emit. Required
//!   attributes such as `cols` on `grid` always emit. Positional arguments
//!   always emit. Those are the `scope` and `ref-gui` ids and the `label`
//!   text.
//! - Attributes emit in field declaration order with `key` last. The
//!   attribute block is omitted entirely when empty.
//!
//! For any tree that decodes without errors and without warnings,
//! [`crate::decode::decode`] of the encoding yields the tree unchanged.

use crate::elem::{ATTRS_MARKER, At, BindPath, Edit, Element, Field, Key, Rgba};
use crate::envelope::{Envelope, Segment};
use gantz_ca::Datum;

/// Encode an element as a datum in canonical form.
///
/// Total. [`Element::Error`] encodes as `(error (@ (reason "...")))`.
/// `error` is not a vocabulary tag, so it decodes back to an unknown-tag
/// error element. Error elements are decode artifacts that stay visible
/// after a store and reload. They are excluded from the round trip law.
pub fn encode(elem: &Element) -> Datum {
    match elem {
        Element::Col(e) => {
            let mut attrs = Vec::new();
            push_f32(&mut attrs, "gap", e.gap);
            push_ident(&mut attrs, "align", e.align.map(|a| a.name()));
            push_key(&mut attrs, &e.key);
            form("col", attrs, Vec::new(), &e.children)
        }
        Element::Row(e) => {
            let mut attrs = Vec::new();
            push_f32(&mut attrs, "gap", e.gap);
            push_ident(&mut attrs, "align", e.align.map(|a| a.name()));
            push_key(&mut attrs, &e.key);
            form("row", attrs, Vec::new(), &e.children)
        }
        Element::Grid(e) => {
            let mut attrs = vec![entry("cols", Datum::I64(i64::from(e.cols)))];
            push_f32(&mut attrs, "gap", e.gap);
            push_key(&mut attrs, &e.key);
            form("grid", attrs, Vec::new(), &e.children)
        }
        Element::Frame(e) => {
            let mut attrs = Vec::new();
            push_string(&mut attrs, "title", &e.title);
            push_key(&mut attrs, &e.key);
            form("frame", attrs, Vec::new(), &e.children)
        }
        Element::Sep(e) => {
            let mut attrs = Vec::new();
            push_key(&mut attrs, &e.key);
            form("sep", attrs, Vec::new(), &[])
        }
        Element::Space(e) => {
            let mut attrs = Vec::new();
            push_key(&mut attrs, &e.key);
            let positional = e
                .amount
                .map(|f| Datum::F64(f64::from(f)))
                .into_iter()
                .collect();
            form("space", attrs, positional, &[])
        }
        Element::Scope(e) => {
            let mut attrs = Vec::new();
            push_key(&mut attrs, &e.key);
            form("scope", attrs, vec![node_id(e.id)], &e.children)
        }
        Element::Dialer(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_at(&mut attrs, &e.at);
            push_field(&mut attrs, &e.field);
            push_on(&mut attrs, &e.on);
            push_f64(&mut attrs, "value", e.value);
            push_f64(&mut attrs, "min", e.min);
            push_f64(&mut attrs, "max", e.max);
            push_f64(&mut attrs, "step", e.step);
            push_int(&mut attrs, "precision", e.precision.map(i64::from));
            push_string(&mut attrs, "label", &e.label);
            push_string(&mut attrs, "prefix", &e.prefix);
            push_string(&mut attrs, "suffix", &e.suffix);
            push_bool(&mut attrs, "push", e.push, true);
            push_ident(&mut attrs, "style", e.style.map(|s| s.name()));
            push_string(&mut attrs, "tip", &e.tip);
            push_key(&mut attrs, &e.key);
            form("dialer", attrs, Vec::new(), &[])
        }
        Element::Toggle(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_at(&mut attrs, &e.at);
            push_field(&mut attrs, &e.field);
            push_on(&mut attrs, &e.on);
            push_bool(&mut attrs, "value", e.value, false);
            push_string(&mut attrs, "label", &e.label);
            push_bool(&mut attrs, "push", e.push, true);
            push_bool(&mut attrs, "pill", e.pill, false);
            push_string(&mut attrs, "tip", &e.tip);
            push_key(&mut attrs, &e.key);
            form("toggle", attrs, Vec::new(), &[])
        }
        Element::Bound(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_at(&mut attrs, &e.at);
            push_field(&mut attrs, &e.field);
            push_on(&mut attrs, &e.on);
            push_f64(&mut attrs, "value", e.value);
            push_string(&mut attrs, "label", &e.label);
            push_key(&mut attrs, &e.key);
            form("bound", attrs, Vec::new(), &[])
        }
        Element::Choice(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_at(&mut attrs, &e.at);
            push_field(&mut attrs, &e.field);
            push_on(&mut attrs, &e.on);
            push_string(&mut attrs, "value", &e.value);
            if !e.options.is_empty() {
                let options = e.options.iter().map(|o| Datum::Sym(o.clone()));
                attrs.push(entry("options", Datum::Seq(options.collect())));
            }
            if !e.tips.is_empty() {
                let tips = e.tips.iter().map(|t| Datum::Str(t.clone()));
                attrs.push(entry("tips", Datum::Seq(tips.collect())));
            }
            push_key(&mut attrs, &e.key);
            form("choice", attrs, Vec::new(), &[])
        }
        Element::TextInput(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_at(&mut attrs, &e.at);
            push_field(&mut attrs, &e.field);
            push_on(&mut attrs, &e.on);
            push_string(&mut attrs, "value", &e.value);
            push_string(&mut attrs, "hint", &e.hint);
            push_bool(&mut attrs, "mono", e.mono, false);
            push_bool(&mut attrs, "push", e.push, true);
            push_string(&mut attrs, "tip", &e.tip);
            push_key(&mut attrs, &e.key);
            form("text-input", attrs, Vec::new(), &[])
        }
        Element::Color(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_at(&mut attrs, &e.at);
            push_field(&mut attrs, &e.field);
            push_on(&mut attrs, &e.on);
            push_color(&mut attrs, "value", e.value);
            push_bool(&mut attrs, "theme", e.theme, true);
            push_string(&mut attrs, "tip", &e.tip);
            push_key(&mut attrs, &e.key);
            form("color", attrs, Vec::new(), &[])
        }
        Element::File(e) => {
            let mut attrs = Vec::new();
            push_on(&mut attrs, &e.on);
            push_string(&mut attrs, "import", &e.import);
            push_string(&mut attrs, "label", &e.label);
            push_string(&mut attrs, "tip", &e.tip);
            push_key(&mut attrs, &e.key);
            form("file", attrs, Vec::new(), &[])
        }
        Element::Button(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_on(&mut attrs, &e.on);
            push_string(&mut attrs, "label", &e.label);
            push_string(&mut attrs, "tip", &e.tip);
            push_key(&mut attrs, &e.key);
            form("button", attrs, Vec::new(), &[])
        }
        Element::Matrix(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_f32(&mut attrs, "cell-size", e.cell_size);
            push_bool(&mut attrs, "push", e.push, true);
            push_key(&mut attrs, &e.key);
            form("matrix", attrs, Vec::new(), &[])
        }
        Element::Canvas(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_at(&mut attrs, &e.at);
            push_f32(&mut attrs, "w", e.w);
            push_f32(&mut attrs, "h", e.h);
            push_bool(&mut attrs, "grid", e.grid, false);
            push_bool(&mut attrs, "axes", e.axes, false);
            push_bool(&mut attrs, "interactive", e.interactive, false);
            push_key(&mut attrs, &e.key);
            form("canvas", attrs, Vec::new(), &[])
        }
        Element::Envelope(e) => {
            let mut attrs = Vec::new();
            push_on(&mut attrs, &e.on);
            if let Some(env) = &e.value {
                attrs.push(entry("value", envelope(env)));
            }
            push_range(&mut attrs, "x-range", e.x_range);
            push_range(&mut attrs, "y-range", e.y_range);
            push_bool(&mut attrs, "grid", e.grid, false);
            push_bool(&mut attrs, "axes", e.axes, false);
            push_f32(&mut attrs, "w", e.w);
            push_f32(&mut attrs, "h", e.h);
            push_key(&mut attrs, &e.key);
            form("envelope", attrs, Vec::new(), &[])
        }
        Element::Label(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_at(&mut attrs, &e.at);
            push_f32(&mut attrs, "size", e.size);
            push_color(&mut attrs, "color", e.color);
            push_ident(&mut attrs, "tone", e.tone.map(|t| t.name()));
            push_bool(&mut attrs, "mono", e.mono, false);
            push_string(&mut attrs, "tip", &e.tip);
            push_key(&mut attrs, &e.key);
            form("label", attrs, vec![Datum::Str(e.text.clone())], &[])
        }
        Element::Value(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_bool(&mut attrs, "wrap", e.wrap, false);
            push_key(&mut attrs, &e.key);
            form("value", attrs, Vec::new(), &[])
        }
        Element::Plot(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_ident(&mut attrs, "mode", e.mode.map(|m| m.name()));
            push_ident(&mut attrs, "style", e.style.map(|s| s.name()));
            push_color(&mut attrs, "color", e.color);
            push_bool(&mut attrs, "grid", e.grid, false);
            push_bool(&mut attrs, "axes", e.axes, false);
            push_bool(&mut attrs, "interactive", e.interactive, false);
            push_f32(&mut attrs, "y-min", e.y_min);
            push_f32(&mut attrs, "y-max", e.y_max);
            push_f32(&mut attrs, "w", e.w);
            push_f32(&mut attrs, "h", e.h);
            push_int(&mut attrs, "capacity", e.capacity.map(i64::from));
            push_key(&mut attrs, &e.key);
            form("plot", attrs, Vec::new(), &[])
        }
        Element::RefGui(e) => {
            let mut attrs = Vec::new();
            push_key(&mut attrs, &e.key);
            form("ref-gui", attrs, vec![node_id(e.id)], &[])
        }
        Element::Error(e) => {
            let attrs = vec![entry("reason", Datum::Str(e.reason.to_string()))];
            form("error", attrs, Vec::new(), &[])
        }
    }
}

/// Assemble `(tag attrs? positional ... child ...)`, omitting the attribute
/// block when empty.
fn form(tag: &str, attrs: Vec<Datum>, positional: Vec<Datum>, children: &[Element]) -> Datum {
    let mut items = vec![ident(tag)];
    if !attrs.is_empty() {
        let mut block = vec![ident(ATTRS_MARKER)];
        block.extend(attrs);
        items.push(Datum::Seq(block));
    }
    items.extend(positional);
    items.extend(children.iter().map(encode));
    Datum::Seq(items)
}

fn ident(s: &str) -> Datum {
    Datum::Sym(s.to_string())
}

/// A `(name value)` attribute entry.
fn entry(name: &str, value: Datum) -> Datum {
    Datum::Seq(vec![ident(name), value])
}

/// A node id as an integer. Ids never approach the `i64` range in practice.
/// Saturation keeps the encoder total regardless.
fn node_id(id: usize) -> Datum {
    Datum::I64(i64::try_from(id).unwrap_or(i64::MAX))
}

fn push_f32(attrs: &mut Vec<Datum>, name: &str, v: Option<f32>) {
    if let Some(f) = v {
        attrs.push(entry(name, Datum::F64(f64::from(f))));
    }
}

fn push_f64(attrs: &mut Vec<Datum>, name: &str, v: Option<f64>) {
    if let Some(f) = v {
        attrs.push(entry(name, Datum::F64(f)));
    }
}

fn push_int(attrs: &mut Vec<Datum>, name: &str, v: Option<i64>) {
    if let Some(i) = v {
        attrs.push(entry(name, Datum::I64(i)));
    }
}

fn push_string(attrs: &mut Vec<Datum>, name: &str, v: &Option<String>) {
    if let Some(s) = v {
        attrs.push(entry(name, Datum::Str(s.clone())));
    }
}

fn push_on(attrs: &mut Vec<Datum>, v: &Option<Edit>) {
    if let Some(Edit(name)) = v {
        attrs.push(entry("on", Datum::Str(name.clone())));
    }
}

fn push_at(attrs: &mut Vec<Datum>, v: &Option<At>) {
    if let Some(At(keys)) = v {
        let keys = keys.iter().map(|k| Datum::Sym(k.clone())).collect();
        attrs.push(entry("at", Datum::Seq(keys)));
    }
}

fn push_field(attrs: &mut Vec<Datum>, v: &Option<Field>) {
    if let Some(Field(name)) = v {
        attrs.push(entry("field", Datum::Str(name.clone())));
    }
}

fn push_ident(attrs: &mut Vec<Datum>, name: &str, v: Option<&'static str>) {
    if let Some(s) = v {
        attrs.push(entry(name, ident(s)));
    }
}

fn push_color(attrs: &mut Vec<Datum>, name: &str, v: Option<Rgba>) {
    if let Some(c) = v {
        attrs.push(entry(name, Datum::Str(c.to_string())));
    }
}

/// Emit a boolean attribute only when it differs from its default.
fn push_bool(attrs: &mut Vec<Datum>, name: &str, v: bool, default: bool) {
    if v != default {
        attrs.push(entry(name, Datum::Bool(v)));
    }
}

fn push_key(attrs: &mut Vec<Datum>, key: &Option<Key>) {
    if let Some(k) = key {
        let v = match k {
            Key::Str(s) => Datum::Str(s.clone()),
            Key::Int(i) => Datum::I64(*i),
        };
        attrs.push(entry("key", v));
    }
}

fn push_range(attrs: &mut Vec<Datum>, name: &str, v: Option<[f32; 2]>) {
    if let Some(range) = v {
        let items = range.map(|x| Datum::F64(f64::from(x)));
        attrs.push(entry(name, Datum::Seq(items.to_vec())));
    }
}

/// An envelope's form, `(init ((level time shape curve) ...) release)`.
fn envelope(env: &Envelope) -> Datum {
    let float = |x: f32| Datum::F64(f64::from(x));
    let segment = |s: &Segment| {
        let items = vec![
            float(s.level),
            float(s.time),
            ident(s.shape.name()),
            float(s.curve),
        ];
        Datum::Seq(items)
    };
    let release = match env.release {
        Some(point) => node_id(point),
        None => Datum::Bool(false),
    };
    let segments = Datum::Seq(env.segments.iter().map(segment).collect());
    Datum::Seq(vec![float(env.init), segments, release])
}

fn push_bind(attrs: &mut Vec<Datum>, bind: &Option<BindPath>) {
    if let Some(BindPath(ids)) = bind {
        let items = ids.iter().map(|&id| node_id(id)).collect();
        attrs.push(entry("bind", Datum::Seq(items)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{Decoded, Limits, decode};
    use crate::diag::{ErrorReason, TreePath};
    use crate::elem::{
        Align, Bound, Button, Canvas, Choice, Col, Color, Dialer, DialerStyle, EnvelopeElem,
        ErrorElem, File, Frame, Grid, Label, Matrix, Plot, PlotMode, PlotStyle, RefGui, Row, Scope,
        Sep, Space, TextInput, Toggle, Tone, Value,
    };

    fn roundtrips(elem: Element) {
        let d = decode(encode(&elem), &Limits::default());
        assert_eq!(
            d,
            Decoded {
                root: elem,
                warnings: vec![],
            }
        );
    }

    #[test]
    fn default_leaves_encode_with_no_attr_block() {
        assert_eq!(
            encode(&Element::Dialer(Dialer::default())),
            Datum::Seq(vec![ident("dialer")])
        );
        assert_eq!(
            encode(&Element::Sep(Sep::default())),
            Datum::Seq(vec![ident("sep")])
        );
        assert_eq!(
            encode(&Element::Space(Space::default())),
            Datum::Seq(vec![ident("space")])
        );
    }

    #[test]
    fn required_forms_always_emit() {
        // The required cols attribute.
        assert_eq!(
            encode(&Element::Grid(Grid::default())),
            Datum::Seq(vec![
                ident("grid"),
                Datum::Seq(vec![ident(ATTRS_MARKER), entry("cols", Datum::I64(1)),]),
            ])
        );
        // The positional label text, even when empty.
        assert_eq!(
            encode(&Element::Label(Label::default())),
            Datum::Seq(vec![ident("label"), Datum::Str(String::new())])
        );
    }

    #[test]
    fn attrs_emit_in_declaration_order_with_key_last() {
        let dialer = Element::Dialer(Dialer {
            bind: Some(BindPath(vec![2])),
            at: None,
            field: None,
            on: None,
            value: None,
            min: Some(0.0),
            max: Some(1.0),
            push: false,
            key: Some(Key::Int(1)),
            ..Default::default()
        });
        let expected = Datum::Seq(vec![
            ident("dialer"),
            Datum::Seq(vec![
                ident(ATTRS_MARKER),
                entry("bind", Datum::Seq(vec![Datum::I64(2)])),
                entry("min", Datum::F64(0.0)),
                entry("max", Datum::F64(1.0)),
                entry("push", Datum::Bool(false)),
                entry("key", Datum::I64(1)),
            ]),
        ]);
        assert_eq!(encode(&dialer), expected);
    }

    #[test]
    fn error_elements_encode_visibly_but_do_not_round_trip() {
        let err = Element::Error(ErrorElem {
            path: TreePath(vec![1]),
            reason: ErrorReason::UnknownTag("bogus".to_string()),
        });
        let encoded = encode(&err);
        let Datum::Seq(items) = &encoded else {
            panic!("expected a list");
        };
        assert_eq!(items[0], ident("error"));
        let d = decode(encoded, &Limits::default());
        assert!(matches!(
            d.root,
            Element::Error(ErrorElem {
                reason: ErrorReason::UnknownTag(ref tag),
                ..
            }) if tag == "error"
        ));
    }

    #[test]
    fn every_element_round_trips_fully_populated() {
        roundtrips(Element::Col(Col {
            gap: Some(4.0),
            align: Some(Align::Center),
            key: Some(Key::Str("c".to_string())),
            children: vec![Element::Sep(Sep::default())],
        }));
        roundtrips(Element::Row(Row {
            gap: Some(2.5),
            align: Some(Align::End),
            key: None,
            children: vec![Element::Space(Space {
                amount: Some(8.0),
                key: None,
            })],
        }));
        roundtrips(Element::Grid(Grid {
            cols: 3,
            gap: Some(1.0),
            key: Some(Key::Int(2)),
            children: vec![Element::Sep(Sep::default())],
        }));
        roundtrips(Element::Frame(Frame {
            title: Some("filter".to_string()),
            key: None,
            children: vec![Element::Button(Button {
                bind: Some(BindPath(vec![7])),
                on: None,
                label: Some("ping".to_string()),
                tip: Some("send a ping".into()),
                key: None,
            })],
        }));
        roundtrips(Element::Scope(Scope {
            id: 3,
            key: None,
            children: vec![Element::Toggle(Toggle {
                bind: Some(BindPath(vec![5])),
                at: None,
                field: None,
                on: None,
                value: false,
                label: Some("drive".to_string()),
                push: false,
                pill: true,
                tip: Some("drive the input".into()),
                key: None,
            })],
        }));
        roundtrips(Element::Dialer(Dialer {
            bind: Some(BindPath(vec![0, 2])),
            at: None,
            field: None,
            on: None,
            value: None,
            min: Some(20.0),
            max: Some(20_000.0),
            step: Some(0.5),
            precision: Some(2),
            label: Some("cutoff".to_string()),
            prefix: Some("!".to_string()),
            suffix: None,
            push: false,
            style: Some(DialerStyle::Knob),
            tip: Some("the cutoff".into()),
            key: Some(Key::Str("k".to_string())),
        }));
        roundtrips(Element::Matrix(Matrix {
            bind: Some(BindPath(vec![1])),
            cell_size: Some(12.0),
            push: false,
            key: None,
        }));
        roundtrips(Element::Label(Label {
            bind: None,
            at: None,
            text: "hello".to_string(),
            size: Some(18.0),
            color: Some(Rgba([255, 0, 128, 64])),
            tone: Some(Tone::Warn),
            mono: true,
            tip: Some("a greeting".into()),
            key: None,
        }));
        roundtrips(Element::Value(Value {
            bind: Some(BindPath(vec![3])),
            wrap: true,
            key: None,
        }));
        roundtrips(Element::Plot(Plot {
            bind: Some(BindPath(vec![4])),
            mode: Some(PlotMode::Scope),
            style: Some(PlotStyle::Line),
            color: Some(Rgba([0, 255, 0, 255])),
            grid: true,
            axes: true,
            interactive: true,
            y_min: Some(-1.0),
            y_max: Some(1.0),
            w: Some(120.0),
            h: Some(80.0),
            capacity: Some(64),
            key: None,
        }));
        roundtrips(Element::Canvas(Canvas {
            at: None,
            bind: Some(BindPath(vec![5])),
            grid: true,
            axes: true,
            interactive: true,
            w: Some(240.0),
            h: Some(60.0),
            key: Some(Key::Str("c".into())),
        }));
        roundtrips(Element::RefGui(RefGui {
            id: 9,
            key: Some(Key::Int(4)),
        }));
    }

    #[test]
    fn every_element_round_trips_at_defaults() {
        roundtrips(Element::Col(Col::default()));
        roundtrips(Element::Row(Row::default()));
        roundtrips(Element::Grid(Grid::default()));
        roundtrips(Element::Frame(Frame::default()));
        roundtrips(Element::Sep(Sep::default()));
        roundtrips(Element::Space(Space::default()));
        roundtrips(Element::Dialer(Dialer::default()));
        roundtrips(Element::Toggle(Toggle::default()));
        roundtrips(Element::Button(Button::default()));
        roundtrips(Element::Matrix(Matrix::default()));
        roundtrips(Element::Label(Label::default()));
        roundtrips(Element::Value(Value::default()));
        roundtrips(Element::Plot(Plot::default()));
        roundtrips(Element::Canvas(Canvas::default()));
        roundtrips(Element::Scope(Scope {
            id: 0,
            key: None,
            children: vec![],
        }));
        roundtrips(Element::RefGui(RefGui { id: 0, key: None }));
        roundtrips(Element::Bound(Bound {
            bind: Some(BindPath(vec![3])),
            at: None,
            field: None,
            on: None,
            value: None,
            label: Some("minimum".to_string()),
            key: None,
        }));
        roundtrips(Element::Choice(Choice {
            bind: Some(BindPath(vec![6])),
            at: None,
            field: None,
            on: None,
            value: None,
            options: vec!["bars".to_string(), "line".to_string()],
            tips: vec!["draw bars".into(), "draw a line".into()],
            key: None,
        }));
        roundtrips(Element::Color(Color {
            bind: Some(BindPath(vec![7])),
            at: None,
            field: None,
            on: None,
            value: None,
            theme: false,
            tip: Some("the colour".into()),
            key: None,
        }));
        roundtrips(Element::Bound(Bound::default()));
        roundtrips(Element::Choice(Choice::default()));
        roundtrips(Element::Color(Color::default()));
    }

    #[test]
    fn field_bindings_and_text_inputs_round_trip() {
        let field = |name: &str| Some(Field(name.to_string()));
        roundtrips(Element::Dialer(Dialer {
            field: field("amount"),
            ..Dialer::default()
        }));
        roundtrips(Element::Toggle(Toggle {
            field: field("on"),
            ..Toggle::default()
        }));
        roundtrips(Element::Choice(Choice {
            field: field("level"),
            options: vec!["warn".into(), "info".into()],
            ..Choice::default()
        }));
        roundtrips(Element::TextInput(TextInput::default()));
        roundtrips(Element::TextInput(TextInput {
            field: field("src"),
            hint: Some("bd sn".into()),
            mono: true,
            ..TextInput::default()
        }));
    }

    #[test]
    fn canvases_round_trip() {
        roundtrips(Element::Canvas(Canvas::default()));
        roundtrips(Element::Canvas(Canvas {
            at: None,
            bind: Some(BindPath(vec![])),
            w: Some(240.0),
            h: Some(80.0),
            grid: true,
            axes: true,
            interactive: true,
            key: None,
        }));
    }

    /// An envelope round trips with every shape, an open release and the
    /// short segment forms.
    #[test]
    fn envelopes_round_trip() {
        use crate::envelope::{Envelope, Segment, Shape};
        roundtrips(Element::Envelope(EnvelopeElem::default()));
        roundtrips(Element::File(File::default()));
        roundtrips(Element::File(File {
            on: Some(Edit("load".into())),
            import: Some("plyphon.wav".into()),
            label: Some("load".into()),
            tip: Some("a wav file".into()),
            key: None,
        }));
        let segments = Shape::ALL.iter().map(|&s| Segment::new(0.5, 0.25, s));
        let env = Envelope {
            init: 0.1,
            segments: segments.chain([Segment::curved(0.0, 1.0, -4.0)]).collect(),
            release: None,
        };
        roundtrips(Element::Envelope(EnvelopeElem {
            on: Some(Edit("env".into())),
            value: Some(env),
            x_range: Some([0.0, 4.0]),
            ..EnvelopeElem::default()
        }));
        let short = Datum::Seq(vec![Datum::I64(1), Datum::F64(0.5)]);
        let shaped = Datum::Seq(vec![Datum::I64(0), Datum::I64(1), ident("exp")]);
        let segments = Datum::Seq(vec![short, shaped]);
        let value = Datum::Seq(vec![Datum::I64(0), segments, Datum::I64(1)]);
        let expr = form("envelope", vec![entry("value", value)], vec![], &[]);
        let short = decode(expr, &Limits::default());
        let Element::Envelope(EnvelopeElem {
            value: Some(env), ..
        }) = short.root
        else {
            panic!("an envelope: {short:?}");
        };
        assert_eq!(env.segments[0], Segment::new(1.0, 0.5, Shape::Lin));
        assert_eq!(env.segments[1], Segment::new(0.0, 1.0, Shape::Exp));
        assert_eq!(env.release, Some(1));
    }

    #[test]
    fn edit_controls_round_trip() {
        let on = |name: &str| Some(Edit(name.to_string()));
        roundtrips(Element::Dialer(Dialer {
            on: on("start"),
            value: Some(0.25),
            ..Dialer::default()
        }));
        roundtrips(Element::Toggle(Toggle {
            on: on("grid"),
            value: true,
            ..Toggle::default()
        }));
        roundtrips(Element::Choice(Choice {
            on: on("res"),
            value: Some("fit".into()),
            options: vec!["fixed".into(), "fit".into()],
            ..Choice::default()
        }));
        roundtrips(Element::TextInput(TextInput {
            on: on("key/0"),
            value: Some("a".into()),
            ..TextInput::default()
        }));
        roundtrips(Element::Color(Color {
            on: on("color/0"),
            value: Some(Rgba([255, 0, 0, 255])),
            ..Color::default()
        }));
        roundtrips(Element::Button(Button {
            on: on("add-key"),
            label: Some("+".into()),
            ..Button::default()
        }));
        roundtrips(Element::Bound(Bound {
            on: on("y-min"),
            value: Some(-1.0),
            ..Bound::default()
        }));
    }
}
