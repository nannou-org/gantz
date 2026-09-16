//! Both codecs agree. One tree, two runtimes, the same decode.

#![cfg(all(feature = "steel", feature = "datum"))]

use gantz_ui::elem::{
    Align, BindPath, Button, Col, Dialer, DialerStyle, Element, Frame, Grid, Key, Label, Matrix,
    Plot, PlotMode, PlotStyle, RefGui, Rgba, Row, Scope, Sep, Space, Toggle, Value,
};
use gantz_ui::{Decoded, Limits, SExpr, codec};

/// One tree exercising every vocabulary element and attribute.
fn full_tree() -> Element {
    Element::Col(Col {
        gap: Some(4.0),
        align: Some(Align::Start),
        key: Some(Key::Str("root".to_string())),
        children: vec![
            Element::Frame(Frame {
                title: Some("filter".to_string()),
                key: None,
                children: vec![Element::Row(Row {
                    gap: Some(2.0),
                    align: Some(Align::Center),
                    key: None,
                    children: vec![
                        Element::Dialer(Dialer {
                            bind: Some(BindPath(vec![0, 2])),
                            min: Some(20.0),
                            max: Some(20_000.0),
                            step: Some(0.5),
                            precision: Some(2),
                            label: Some("cutoff".to_string()),
                            push: false,
                            style: Some(DialerStyle::Knob),
                            key: Some(Key::Int(1)),
                        }),
                        Element::Toggle(Toggle {
                            bind: Some(BindPath(vec![5])),
                            label: Some("drive".to_string()),
                            push: false,
                            key: None,
                        }),
                        Element::Button(Button {
                            bind: Some(BindPath(vec![7])),
                            label: Some("ping".to_string()),
                            key: None,
                        }),
                    ],
                })],
            }),
            Element::Grid(Grid {
                cols: 4,
                gap: Some(1.0),
                key: None,
                children: vec![
                    Element::Matrix(Matrix {
                        bind: Some(BindPath(vec![8])),
                        cell_size: Some(12.0),
                        push: false,
                        key: None,
                    }),
                    Element::Sep(Sep::default()),
                    Element::Space(Space {
                        amount: Some(8.0),
                        key: None,
                    }),
                    Element::Value(Value {
                        bind: Some(BindPath(vec![3])),
                        wrap: true,
                        key: None,
                    }),
                ],
            }),
            Element::Label(Label {
                text: "hello".to_string(),
                size: Some(18.0),
                color: Some(Rgba([255, 0, 128, 64])),
                key: None,
            }),
            Element::Plot(Plot {
                bind: Some(BindPath(vec![4])),
                mode: Some(PlotMode::Signal),
                style: Some(PlotStyle::Bars),
                color: Some(Rgba([0, 255, 0, 255])),
                grid: true,
                axes: true,
                interactive: true,
                y_min: Some(-1.0),
                y_max: Some(1.0),
                w: Some(120.0),
                h: Some(80.0),
                key: None,
            }),
            Element::Scope(Scope {
                id: 9,
                key: None,
                children: vec![Element::RefGui(RefGui {
                    id: 9,
                    key: Some(Key::Str("child".to_string())),
                })],
            }),
        ],
    })
}

/// Identifiers and strings are interchangeable, so normalizing identifiers
/// away yields the codec-independent form.
fn normalized(expr: SExpr) -> SExpr {
    match expr {
        SExpr::Ident(s) => SExpr::Str(s),
        SExpr::List(items) => SExpr::List(items.into_iter().map(normalized).collect()),
        other => other,
    }
}

#[test]
fn both_codecs_decode_the_same_tree_identically() {
    let tree = full_tree();
    let limits = Limits::default();

    let steel_val = codec::steel::encode(&tree);
    let datum = codec::datum::encode(&tree);

    let via_steel = codec::steel::decode(&steel_val, &limits);
    let via_datum = codec::datum::decode(&datum, &limits);

    let expected = Decoded {
        root: tree,
        warnings: vec![],
    };
    assert_eq!(via_steel, expected);
    assert_eq!(via_datum, expected);
}

#[test]
fn both_encodings_agree_modulo_identifier_normalization() {
    let tree = full_tree();
    let steel_lowered = codec::steel::lower(&codec::steel::encode(&tree));
    let datum_lowered = codec::datum::lower(&codec::datum::encode(&tree));
    assert_eq!(normalized(steel_lowered), datum_lowered);
}
