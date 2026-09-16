//! The canonical encoder from the typed tree to the abstract value model.
//!
//! Canonical form keeps stored GUI values minimal and stable. It mirrors the
//! ext conventions elsewhere in gantz:
//!
//! - Tags and attribute names emit as identifiers.
//! - Only attributes that differ from the element's `Default` emit. Required
//!   attributes such as `cols` on `grid` always emit. Positional arguments
//!   always emit. Those are the `scope` and `ref-gui` ids and the `label`
//!   text.
//! - Attributes emit in field declaration order with `key` last. The
//!   attribute block is omitted entirely when empty.
//!
//! For any tree that decodes without errors and without warnings,
//! [`crate::decode::decode`] of the encoding yields the tree unchanged.

use crate::elem::{ATTRS_MARKER, BindPath, Element, Key, Rgba};
use crate::sexpr::SExpr;

/// Encode an element into the abstract value model in canonical form.
///
/// Total. [`Element::Error`] encodes as `(error (@ (reason "...")))`.
/// `error` is not a vocabulary tag, so it decodes back to an unknown-tag
/// error element. Error elements are decode artifacts that stay visible
/// after a store and reload. They are excluded from the round trip law.
pub fn encode(elem: &Element) -> SExpr {
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
            let mut attrs = vec![entry("cols", SExpr::Int(i64::from(e.cols)))];
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
                .map(|f| SExpr::Float(f64::from(f)))
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
            push_f64(&mut attrs, "min", e.min);
            push_f64(&mut attrs, "max", e.max);
            push_f64(&mut attrs, "step", e.step);
            push_int(&mut attrs, "precision", e.precision.map(i64::from));
            push_string(&mut attrs, "label", &e.label);
            push_bool(&mut attrs, "push", e.push, true);
            push_ident(&mut attrs, "style", e.style.map(|s| s.name()));
            push_key(&mut attrs, &e.key);
            form("dialer", attrs, Vec::new(), &[])
        }
        Element::Toggle(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_string(&mut attrs, "label", &e.label);
            push_bool(&mut attrs, "push", e.push, true);
            push_key(&mut attrs, &e.key);
            form("toggle", attrs, Vec::new(), &[])
        }
        Element::Button(e) => {
            let mut attrs = Vec::new();
            push_bind(&mut attrs, &e.bind);
            push_string(&mut attrs, "label", &e.label);
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
        Element::Label(e) => {
            let mut attrs = Vec::new();
            push_f32(&mut attrs, "size", e.size);
            push_color(&mut attrs, "color", e.color);
            push_key(&mut attrs, &e.key);
            form("label", attrs, vec![SExpr::Str(e.text.clone())], &[])
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
            push_key(&mut attrs, &e.key);
            form("plot", attrs, Vec::new(), &[])
        }
        Element::RefGui(e) => {
            let mut attrs = Vec::new();
            push_key(&mut attrs, &e.key);
            form("ref-gui", attrs, vec![node_id(e.id)], &[])
        }
        Element::Error(e) => {
            let attrs = vec![entry("reason", SExpr::Str(e.reason.to_string()))];
            form("error", attrs, Vec::new(), &[])
        }
    }
}

/// Assemble `(tag attrs? positional ... child ...)`, omitting the attribute
/// block when empty.
fn form(tag: &str, attrs: Vec<SExpr>, positional: Vec<SExpr>, children: &[Element]) -> SExpr {
    let mut items = vec![ident(tag)];
    if !attrs.is_empty() {
        let mut block = vec![ident(ATTRS_MARKER)];
        block.extend(attrs);
        items.push(SExpr::List(block));
    }
    items.extend(positional);
    items.extend(children.iter().map(encode));
    SExpr::List(items)
}

fn ident(s: &str) -> SExpr {
    SExpr::Ident(s.to_string())
}

/// A `(name value)` attribute entry.
fn entry(name: &str, value: SExpr) -> SExpr {
    SExpr::List(vec![ident(name), value])
}

/// A node id as an integer. Ids never approach the `i64` range in practice.
/// Saturation keeps the encoder total regardless.
fn node_id(id: usize) -> SExpr {
    SExpr::Int(i64::try_from(id).unwrap_or(i64::MAX))
}

fn push_f32(attrs: &mut Vec<SExpr>, name: &str, v: Option<f32>) {
    if let Some(f) = v {
        attrs.push(entry(name, SExpr::Float(f64::from(f))));
    }
}

fn push_f64(attrs: &mut Vec<SExpr>, name: &str, v: Option<f64>) {
    if let Some(f) = v {
        attrs.push(entry(name, SExpr::Float(f)));
    }
}

fn push_int(attrs: &mut Vec<SExpr>, name: &str, v: Option<i64>) {
    if let Some(i) = v {
        attrs.push(entry(name, SExpr::Int(i)));
    }
}

fn push_string(attrs: &mut Vec<SExpr>, name: &str, v: &Option<String>) {
    if let Some(s) = v {
        attrs.push(entry(name, SExpr::Str(s.clone())));
    }
}

fn push_ident(attrs: &mut Vec<SExpr>, name: &str, v: Option<&'static str>) {
    if let Some(s) = v {
        attrs.push(entry(name, ident(s)));
    }
}

fn push_color(attrs: &mut Vec<SExpr>, name: &str, v: Option<Rgba>) {
    if let Some(c) = v {
        attrs.push(entry(name, SExpr::Str(c.to_string())));
    }
}

/// Emit a boolean attribute only when it differs from its default.
fn push_bool(attrs: &mut Vec<SExpr>, name: &str, v: bool, default: bool) {
    if v != default {
        attrs.push(entry(name, SExpr::Bool(v)));
    }
}

fn push_key(attrs: &mut Vec<SExpr>, key: &Option<Key>) {
    if let Some(k) = key {
        let v = match k {
            Key::Str(s) => SExpr::Str(s.clone()),
            Key::Int(i) => SExpr::Int(*i),
        };
        attrs.push(entry("key", v));
    }
}

fn push_bind(attrs: &mut Vec<SExpr>, bind: &Option<BindPath>) {
    if let Some(BindPath(ids)) = bind {
        let items = ids.iter().map(|&id| node_id(id)).collect();
        attrs.push(entry("bind", SExpr::List(items)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{Decoded, Limits, decode};
    use crate::diag::{ErrorReason, TreePath};
    use crate::elem::{
        Align, Button, Col, Dialer, DialerStyle, ErrorElem, Frame, Grid, Label, Matrix, Plot,
        PlotMode, PlotStyle, RefGui, Row, Scope, Sep, Space, Toggle, Value,
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
            SExpr::List(vec![ident("dialer")])
        );
        assert_eq!(
            encode(&Element::Sep(Sep::default())),
            SExpr::List(vec![ident("sep")])
        );
        assert_eq!(
            encode(&Element::Space(Space::default())),
            SExpr::List(vec![ident("space")])
        );
    }

    #[test]
    fn required_forms_always_emit() {
        // The required cols attribute.
        assert_eq!(
            encode(&Element::Grid(Grid::default())),
            SExpr::List(vec![
                ident("grid"),
                SExpr::List(vec![ident(ATTRS_MARKER), entry("cols", SExpr::Int(1)),]),
            ])
        );
        // The positional label text, even when empty.
        assert_eq!(
            encode(&Element::Label(Label::default())),
            SExpr::List(vec![ident("label"), SExpr::Str(String::new())])
        );
    }

    #[test]
    fn attrs_emit_in_declaration_order_with_key_last() {
        let dialer = Element::Dialer(Dialer {
            bind: Some(BindPath(vec![2])),
            min: Some(0.0),
            max: Some(1.0),
            push: false,
            key: Some(Key::Int(1)),
            ..Default::default()
        });
        let expected = SExpr::List(vec![
            ident("dialer"),
            SExpr::List(vec![
                ident(ATTRS_MARKER),
                entry("bind", SExpr::List(vec![SExpr::Int(2)])),
                entry("min", SExpr::Float(0.0)),
                entry("max", SExpr::Float(1.0)),
                entry("push", SExpr::Bool(false)),
                entry("key", SExpr::Int(1)),
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
        let SExpr::List(items) = &encoded else {
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
                label: Some("ping".to_string()),
                key: None,
            })],
        }));
        roundtrips(Element::Scope(Scope {
            id: 3,
            key: None,
            children: vec![Element::Toggle(Toggle {
                bind: Some(BindPath(vec![5])),
                label: Some("drive".to_string()),
                push: false,
                key: None,
            })],
        }));
        roundtrips(Element::Dialer(Dialer {
            bind: Some(BindPath(vec![0, 2])),
            min: Some(20.0),
            max: Some(20_000.0),
            step: Some(0.5),
            precision: Some(2),
            label: Some("cutoff".to_string()),
            push: false,
            style: Some(DialerStyle::Knob),
            key: Some(Key::Str("k".to_string())),
        }));
        roundtrips(Element::Matrix(Matrix {
            bind: Some(BindPath(vec![1])),
            cell_size: Some(12.0),
            push: false,
            key: None,
        }));
        roundtrips(Element::Label(Label {
            text: "hello".to_string(),
            size: Some(18.0),
            color: Some(Rgba([255, 0, 128, 64])),
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
            key: None,
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
        roundtrips(Element::Scope(Scope {
            id: 0,
            key: None,
            children: vec![],
        }));
        roundtrips(Element::RefGui(RefGui { id: 0, key: None }));
    }

    #[test]
    fn a_deep_mixed_tree_round_trips() {
        let tree = Element::Col(Col {
            gap: Some(4.0),
            align: None,
            key: None,
            children: vec![
                Element::Frame(Frame {
                    title: Some("filter".to_string()),
                    key: None,
                    children: vec![Element::Row(Row {
                        children: vec![
                            Element::Dialer(Dialer {
                                bind: Some(BindPath(vec![2])),
                                min: Some(20.0),
                                max: Some(20_000.0),
                                label: Some("cutoff".to_string()),
                                ..Default::default()
                            }),
                            Element::Dialer(Dialer {
                                bind: Some(BindPath(vec![3])),
                                min: Some(0.1),
                                max: Some(4.0),
                                label: Some("q".to_string()),
                                ..Default::default()
                            }),
                        ],
                        ..Default::default()
                    })],
                }),
                Element::Row(Row {
                    children: vec![
                        Element::Toggle(Toggle {
                            bind: Some(BindPath(vec![5])),
                            label: Some("drive".to_string()),
                            ..Default::default()
                        }),
                        Element::Button(Button {
                            bind: Some(BindPath(vec![7])),
                            label: Some("ping".to_string()),
                            key: None,
                        }),
                    ],
                    ..Default::default()
                }),
                Element::Scope(Scope {
                    id: 9,
                    key: None,
                    children: vec![Element::Plot(Plot {
                        bind: Some(BindPath(vec![1])),
                        mode: Some(PlotMode::Scope),
                        ..Default::default()
                    })],
                }),
                Element::RefGui(RefGui { id: 9, key: None }),
            ],
        });
        roundtrips(tree);
    }
}
