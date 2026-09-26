//! Datum codec integration tests for the storage-side encoding.

#![cfg(feature = "datum")]

use gantz_core::datum::Datum;
use gantz_ui::codec::datum::{decode, encode};
use gantz_ui::{
    BindPath, Col, Decoded, Dialer, Element, ErrorReason, Frame, Label, Limits, Rgba, Sep, Toggle,
    WarningKind,
};

fn s(text: &str) -> Datum {
    Datum::Str(text.to_string())
}

fn seq(items: Vec<Datum>) -> Datum {
    Datum::Seq(items)
}

fn attr(name: &str, value: Datum) -> Datum {
    seq(vec![s(name), value])
}

#[test]
fn string_keyed_trees_decode() {
    // Tags, attribute names and enum values all arrive as strings from
    // Datum. The tolerance rule makes this identical to the symbol form.
    let tree = seq(vec![
        s("col"),
        seq(vec![s("@"), attr("gap", Datum::I64(4))]),
        seq(vec![
            s("dialer"),
            seq(vec![
                s("@"),
                attr("bind", seq(vec![Datum::I64(0), Datum::I64(2)])),
                attr("min", Datum::F64(0.0)),
                attr("label", s("cutoff")),
            ]),
        ]),
        seq(vec![s("label"), s("filter")]),
    ]);
    let d = decode(&tree, &Limits::default());
    let expected = Element::Col(Col {
        gap: Some(4.0),
        align: None,
        key: None,
        children: vec![
            Element::Dialer(Dialer {
                bind: Some(BindPath(vec![0, 2])),
                min: Some(0.0),
                label: Some("cutoff".to_string()),
                ..Default::default()
            }),
            Element::Label(Label {
                text: "filter".to_string(),
                ..Default::default()
            }),
        ],
    });
    assert_eq!(
        d,
        Decoded {
            root: expected,
            warnings: vec![],
        }
    );
}

#[test]
fn u64_edges() {
    let in_range = seq(vec![
        s("dialer"),
        seq(vec![s("@"), attr("precision", Datum::U64(3))]),
    ]);
    let d = decode(&in_range, &Limits::default());
    let Element::Dialer(dialer) = &d.root else {
        panic!("expected a dialer");
    };
    assert_eq!(dialer.precision, Some(3));
    assert_eq!(d.warnings, vec![]);

    let out_of_range = seq(vec![
        s("dialer"),
        seq(vec![s("@"), attr("min", Datum::U64(u64::MAX))]),
    ]);
    let d = decode(&out_of_range, &Limits::default());
    assert!(matches!(
        d.warnings[0].kind,
        WarningKind::InvalidAttrValue { ref found, .. } if found == "an out-of-range integer"
    ));

    let as_child = seq(vec![s("col"), Datum::U64(u64::MAX)]);
    let d = decode(&as_child, &Limits::default());
    let Element::Col(col) = &d.root else {
        panic!("expected a col");
    };
    assert!(matches!(
        col.children[0],
        Element::Error(ref e)
            if matches!(&e.reason, ErrorReason::NotAnElement { found } if found == "an out-of-range integer")
    ));
}

#[test]
fn foreign_datum_kinds_are_reported_by_name() {
    for (datum, name) in [
        (Datum::Null, "null"),
        (Datum::Char('x'), "a character"),
        (Datum::Bytes(vec![1, 2]), "a byte buffer"),
        (Datum::Map(vec![("k".to_string(), Datum::I64(1))]), "a map"),
    ] {
        let tree = seq(vec![s("col"), datum]);
        let d = decode(&tree, &Limits::default());
        let Element::Col(col) = &d.root else {
            panic!("expected a col");
        };
        assert!(matches!(
            col.children[0],
            Element::Error(ref e)
                if matches!(&e.reason, ErrorReason::NotAnElement { found } if found == name)
        ));
    }
}

#[test]
fn a_full_tree_round_trips_through_datum() {
    let tree = Element::Frame(Frame {
        title: Some("filter".to_string()),
        key: None,
        children: vec![
            Element::Dialer(Dialer {
                bind: Some(BindPath(vec![2])),
                min: Some(20.0),
                max: Some(20_000.0),
                precision: Some(1),
                label: Some("cutoff".to_string()),
                push: false,
                ..Default::default()
            }),
            Element::Sep(Sep::default()),
            Element::Toggle(Toggle {
                bind: Some(BindPath(vec![5])),
                label: Some("drive".to_string()),
                ..Default::default()
            }),
            Element::Label(Label {
                text: "hello".to_string(),
                color: Some(Rgba([255, 0, 128, 255])),
                ..Default::default()
            }),
        ],
    });
    let datum = encode(&tree);
    // Tags normalize to strings at the datum level.
    let Datum::Seq(items) = &datum else {
        panic!("expected a seq");
    };
    assert_eq!(items[0], s("frame"));
    let d = decode(&datum, &Limits::default());
    assert_eq!(
        d,
        Decoded {
            root: tree,
            warnings: vec![],
        }
    );
}
