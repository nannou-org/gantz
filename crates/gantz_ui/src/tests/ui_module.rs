//! The `gantz/ui` Steel module on the prelude-free base engine.

#![cfg(feature = "steel")]

use gantz_core::steel::SteelVal;
use gantz_core::steel::steel_vm::engine::Engine;

fn engine() -> Engine {
    let mut vm = gantz_core::vm::new_engine(crate::modules());
    vm.run("(require \"gantz/ui\")".to_string())
        .expect("require gantz/ui");
    vm
}

/// Assert `expr` evaluates to the quoted `expected` literal.
fn assert_eval(vm: &mut Engine, case: &str, expected: &str, expr: &str) {
    let check = format!("(equal? '{expected} {expr})");
    match vm.run(check).expect("steel error").last() {
        Some(SteelVal::BoolV(true)) => (),
        _ => {
            let actual = vm.run(expr.to_string()).expect("steel error");
            panic!("{case}: expected {expected}\n     got {actual:?}\n     for {expr}");
        }
    }
}

#[test]
fn ui_module_helpers_build_elements() {
    let cases = [
        (
            "all absent yields the bare tag",
            "(row)",
            "(ui-elem 'row '() (list (list 'gap (None))) (None))",
        ),
        (
            "present attrs keep order and drop absent",
            "(row (@ (gap 4) (align \"end\")))",
            "(ui-elem 'row '() (list (list 'gap (Some 4)) (list 'x (None)) (list 'align (Some \"end\"))) (None))",
        ),
        (
            "positionals drop absent",
            "(label \"hi\" (@ (size 12.0)))",
            "(ui-elem 'label (list (Some \"hi\") (None)) (list (list 'size (Some 12.0))) (None))",
        ),
        (
            "child list is spliced",
            "(col (sep) (label \"a\"))",
            "(ui-elem 'col '() '() (Some (list '(sep) '(label \"a\"))))",
        ),
        (
            "single child is wrapped",
            "(col (sep))",
            "(ui-elem 'col '() '() (Some '(sep)))",
        ),
        (
            "empty child list yields no children",
            "(col)",
            "(ui-elem 'col '() '() (Some '()))",
        ),
        (
            "ui-int rounds to exact",
            "(grid (@ (cols 3)))",
            "(ui-elem 'grid '() (list (list 'cols (ui-int (Some 2.6)))) (None))",
        ),
        (
            "ui-int of none is absent",
            "(grid)",
            "(ui-elem 'grid '() (list (list 'cols (ui-int (None)))) (None))",
        ),
        (
            "ui-id takes the first segment",
            "(ref-gui 2)",
            "(ui-elem 'ref-gui (list (ui-id (Some '(2 5)))) '() (None))",
        ),
        (
            "ui-id of none is absent",
            "(ref-gui)",
            "(ui-elem 'ref-gui (list (ui-id (None))) '() (None))",
        ),
    ];
    let mut vm = engine();
    for (case, expected, expr) in cases {
        assert_eval(&mut vm, case, expected, expr);
    }
}
