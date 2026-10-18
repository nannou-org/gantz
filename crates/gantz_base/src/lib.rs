//! Embedded base node export for gantz.
//!
//! Ships the `base.gantz` file as a compile-time constant. Downstream crates
//! do not need fragile relative `include_bytes!` paths.

/// Raw bytes of the baked-in base `.gantz` export, embedded at compile time.
pub const BYTES: &[u8] = include_bytes!("../base.gantz");

/// Earlier names of the base graphs, each as `(old, new)`. A host renames
/// stored graphs and references that still use an old name.
pub const RENAMES: &[(&str, &str)] = &[
    ("demo-all", "demo.all"),
    ("demo-arithmetic", "demo.arithmetic"),
    ("demo-comparison", "demo.comparison"),
    ("demo-gui", "demo.gui"),
    ("demo-gui-compose", "demo.gui-compose"),
    ("demo-list", "demo.list"),
    ("demo-list-fn", "demo.list-fn"),
    ("demo-logic", "demo.logic"),
    ("demo-predicate", "demo.predicate"),
    ("uibutton", "ui.button"),
    ("uicol", "ui.col"),
    ("uidialer", "ui.dialer"),
    ("uiframe", "ui.frame"),
    ("uigrid", "ui.grid"),
    ("uilabel", "ui.label"),
    ("uimatrix", "ui.matrix"),
    ("uiplot", "ui.plot"),
    ("uiref", "ui.ref"),
    ("uirow", "ui.row"),
    ("uisep", "ui.sep"),
    ("uispace", "ui.space"),
    ("uitoggle", "ui.toggle"),
    ("uivalue", "ui.value"),
];
