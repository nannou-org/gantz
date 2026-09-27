//! Steel codec integration tests driving the real reader.

#![cfg(feature = "steel")]

use crate::codec::steel::decode;
use crate::{
    Align, BindPath, Col, Decoded, Dialer, Element, Label, Limits, Row, Toggle, WarningKind,
};
use steel::SteelVal;
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
