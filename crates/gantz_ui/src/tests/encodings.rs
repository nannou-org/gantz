//! One tree through every encoding: a datum, and a Steel value.

#![cfg(feature = "steel")]

use crate::elem::{
    Align, BindPath, Bound, Button, Choice, Col, Color, Dialer, DialerStyle, Element, Frame, Grid,
    Key, Label, Matrix, Plot, PlotMode, PlotStyle, RefGui, Rgba, Row, Scope, Sep, Space, Toggle,
    Value,
};
use crate::{Decoded, Limits, codec};

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
                            prefix: Some("!".to_string()),
                            push: false,
                            style: Some(DialerStyle::Knob),
                            key: Some(Key::Int(1)),
                        }),
                        Element::Toggle(Toggle {
                            bind: Some(BindPath(vec![5])),
                            label: Some("drive".to_string()),
                            push: false,
                            pill: true,
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
                    Element::Bound(Bound {
                        bind: Some(BindPath(vec![10])),
                        label: Some("level".to_string()),
                        key: None,
                    }),
                    Element::Choice(Choice {
                        bind: Some(BindPath(vec![11])),
                        options: vec!["sine".to_string(), "saw".to_string()],
                        key: None,
                    }),
                    Element::Color(Color {
                        bind: Some(BindPath(vec![12])),
                        key: Some(Key::Int(2)),
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
                capacity: Some(256),
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

/// The tree survives its datum encoding and a trip through Steel, and the
/// Steel value reads back as the same datum.
#[test]
fn the_full_tree_round_trips_through_every_encoding() {
    let tree = full_tree();
    let limits = Limits::default();
    let expected = Decoded {
        root: tree.clone(),
        warnings: vec![],
    };
    let datum = crate::encode(&tree);
    assert_eq!(crate::decode(datum.clone(), &limits), expected);
    let steel = codec::steel::encode(&tree);
    assert_eq!(codec::steel::decode(&steel, &limits), expected);
    assert_eq!(codec::steel::lower(&steel), datum);
}
