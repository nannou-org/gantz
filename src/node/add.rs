#[derive(GantzNode_)]
#[inlets = r#"
    #[derive(Debug)]
    struct Inlets {
        left: f64,
        right: f64,
    }
"#]
#[outlets = r#"
    enum Outlets {
        Result(f64),
    }
"#]
#[process_inlet(left = "AddF64::process_left")]
#[process_outlet(Result = "AddF64::process_result")]
struct AddF64;

impl AddF64 {
    fn process_left(&mut self, inlet: &mut f64, value: &f64) {
        *inlet = *value;
    }

    fn process_result(&mut self, inlets: &Inlets) -> f64 {
        inlets.left.clone() + inlets.right.clone()
    }
}
