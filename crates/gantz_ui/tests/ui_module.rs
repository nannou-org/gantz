//! The `gantz/ui` Steel module on the prelude-free base engine.

#![cfg(feature = "steel")]

use gantz_core::steel::SteelVal;
use gantz_core::steel::steel_vm::engine::Engine;

fn engine() -> Engine {
    let mut vm = gantz_core::vm::new_engine(gantz_ui::modules());
    vm.run("(require \"gantz/ui\")".to_string())
        .expect("require gantz/ui");
    vm
}

/// Assert `expr` evaluates to the quoted `expected` literal.
fn assert_eval(expected: &str, expr: &str) {
    let mut vm = engine();
    let check = format!("(equal? '{expected} {expr})");
    match vm.run(check).expect("steel error").last() {
        Some(SteelVal::BoolV(true)) => (),
        _ => {
            let actual = vm.run(expr.to_string()).expect("steel error");
            panic!("expected {expected}\n     got {actual:?}\n     for {expr}");
        }
    }
}

#[test]
fn all_absent_yields_the_bare_tag() {
    assert_eval(
        "(row)",
        "(ui-elem 'row '() (list (list 'gap (None))) (None))",
    );
}

#[test]
fn present_attrs_keep_order_and_drop_absent() {
    assert_eval(
        "(row (@ (gap 4) (align \"end\")))",
        "(ui-elem 'row '() (list (list 'gap (Some 4)) (list 'x (None)) (list 'align (Some \"end\"))) (None))",
    );
}

#[test]
fn positionals_drop_absent() {
    assert_eval(
        "(label \"hi\" (@ (size 12.0)))",
        "(ui-elem 'label (list (Some \"hi\") (None)) (list (list 'size (Some 12.0))) (None))",
    );
}

#[test]
fn child_list_is_spliced() {
    assert_eval(
        "(col (sep) (label \"a\"))",
        "(ui-elem 'col '() '() (Some (list '(sep) '(label \"a\"))))",
    );
}

#[test]
fn single_child_is_wrapped() {
    assert_eval("(col (sep))", "(ui-elem 'col '() '() (Some '(sep)))");
    assert_eval("(col)", "(ui-elem 'col '() '() (Some '()))");
}

#[test]
fn ui_int_rounds_to_exact() {
    assert_eval(
        "(grid (@ (cols 3)))",
        "(ui-elem 'grid '() (list (list 'cols (ui-int (Some 2.6)))) (None))",
    );
    assert_eval(
        "(grid)",
        "(ui-elem 'grid '() (list (list 'cols (ui-int (None)))) (None))",
    );
}

#[test]
fn ui_id_takes_the_first_segment() {
    assert_eval(
        "(ref-gui 2)",
        "(ui-elem 'ref-gui (list (ui-id (Some '(2 5)))) '() (None))",
    );
    assert_eval(
        "(ref-gui)",
        "(ui-elem 'ref-gui (list (ui-id (None))) '() (None))",
    );
}
