//! Steel codec integration tests driving the real reader.

#![cfg(feature = "steel")]

use gantz_ui::codec::steel::{decode, encode, lower};
use gantz_ui::{
    Align, BindPath, Col, Decoded, Dialer, Element, ErrorReason, Frame, Label, Limits, Row, SExpr,
    Sep, Toggle, WarningKind,
};
use steel::SteelVal;
use steel::gc::Gc;
use steel::steel_vm::engine::Engine;

fn run(src: &str) -> SteelVal {
    Engine::new_base()
        .run(src.to_string())
        .unwrap()
        .last()
        .cloned()
        .expect("no value")
}

fn sym(s: &str) -> SteelVal {
    SteelVal::SymbolV(s.into())
}

fn slist(items: Vec<SteelVal>) -> SteelVal {
    SteelVal::ListV(items.into_iter().collect())
}

fn svec(items: Vec<SteelVal>) -> SteelVal {
    let v: steel::Vector<SteelVal> = items.into_iter().collect();
    SteelVal::VectorV(Gc::new(v).into())
}

#[test]
fn reader_output_decodes() {
    let val = run(r#"'(col (@ (gap 4) (align center))
             (row (dialer (@ (bind (0 2)) (min 0.0) (max 20000.0) (label "cutoff")))
                  (toggle (@ (bind (5)) (label "drive") (push #f))))
             (label "filter"))"#);
    let d = decode(&val, &Limits::default());
    let expected = Element::Col(Col {
        gap: Some(4.0),
        align: Some(Align::Center),
        key: None,
        children: vec![
            Element::Row(Row {
                children: vec![
                    Element::Dialer(Dialer {
                        bind: Some(BindPath(vec![0, 2])),
                        min: Some(0.0),
                        max: Some(20_000.0),
                        label: Some("cutoff".to_string()),
                        ..Default::default()
                    }),
                    Element::Toggle(Toggle {
                        bind: Some(BindPath(vec![5])),
                        label: Some("drive".to_string()),
                        push: false,
                        key: None,
                    }),
                ],
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
fn string_tags_decode_like_symbol_tags() {
    let strings = run(r#"'("col" ("sep"))"#);
    let symbols = run("'(col (sep))");
    let limits = Limits::default();
    assert_eq!(decode(&strings, &limits), decode(&symbols, &limits));
}

#[test]
fn vectors_decode_like_lists() {
    let as_list = run("'(col (sep) (sep))");
    let sep = svec(vec![sym("sep")]);
    let as_vec = svec(vec![sym("col"), sep.clone(), sep]);
    let limits = Limits::default();
    assert_eq!(decode(&as_vec, &limits), decode(&as_list, &limits));
}

#[test]
fn intv_edge_values_lower_losslessly() {
    assert_eq!(
        lower(&SteelVal::IntV(isize::MAX)),
        SExpr::Int(isize::MAX as i64)
    );
    assert_eq!(
        lower(&SteelVal::IntV(isize::MIN)),
        SExpr::Int(isize::MIN as i64)
    );
}

#[test]
fn non_finite_numbers_warn_as_attr_values() {
    let tree = slist(vec![
        sym("dialer"),
        slist(vec![
            sym("@"),
            slist(vec![sym("min"), SteelVal::NumV(f64::NAN)]),
        ]),
    ]);
    let d = decode(&tree, &Limits::default());
    assert_eq!(d.root, Element::Dialer(Dialer::default()));
    assert!(matches!(
        d.warnings[0].kind,
        WarningKind::InvalidAttrValue { ref found, .. } if found == "a non-finite number"
    ));
}

#[test]
fn foreign_values_in_element_position_error() {
    let closure = run("(lambda (x) x)");
    let tree = slist(vec![sym("col"), closure]);
    let d = decode(&tree, &Limits::default());
    let Element::Col(col) = &d.root else {
        panic!("expected a col");
    };
    let Element::Error(err) = &col.children[0] else {
        panic!("expected an error element");
    };
    assert!(matches!(err.reason, ErrorReason::NotAnElement { .. }));
}

#[test]
fn a_full_tree_round_trips_through_steel() {
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
                ..Default::default()
            }),
            Element::Sep(Sep::default()),
            Element::Toggle(Toggle {
                bind: Some(BindPath(vec![5])),
                label: Some("drive".to_string()),
                push: false,
                key: None,
            }),
        ],
    });
    let val = encode(&tree);
    // Canonical encoding uses symbols for tags and attribute names.
    let SteelVal::ListV(items) = &val else {
        panic!("expected a list");
    };
    assert_eq!(items.iter().next(), Some(&sym("frame")));
    let d = decode(&val, &Limits::default());
    assert_eq!(
        d,
        Decoded {
            root: tree,
            warnings: vec![],
        }
    );
}
