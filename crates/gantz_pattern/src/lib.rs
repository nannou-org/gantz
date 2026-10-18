//! A pattern domain for gantz.
//!
//! A pattern is a Steel closure from a span, a pair of rational time
//! points, to the list of events occurring along it. Higher-order
//! composition builds intricate patterns from simple constructors. The
//! vocabulary lives in the [`MODULE`] Steel module so patterns and their
//! combinators compose as ordinary closures inside the VM, and user
//! functions such as graphs lifted via `fn-ref` apply directly.
//!
//! Events carry an `active` span where the value applies within the
//! query, and a `whole` span carrying the event's full structure. Whole
//! is absent for continuous signals.

pub mod mini;
pub mod node;
pub mod sugar;

#[cfg(feature = "egui")]
pub mod egui;

#[cfg(feature = "egui")]
pub use self::egui::Pplot;
pub use node::{Pmini, builtins};
pub use sugar::PatternSugar;

use gantz_core::vm::SteelModule;

/// The `gantz/pattern` Steel module.
///
/// It requires `gantz/rng`, so register it with the modules from
/// [`modules`]. Register via [`gantz_core::vm::new_engine`] or the app's
/// steel-module collection, then `(require "gantz/pattern")` to use. All
/// provided names carry the `pat/` prefix.
pub const MODULE: SteelModule = SteelModule::new("gantz/pattern", include_str!("pattern.scm"));

/// The Steel modules this domain needs: `gantz/rng` and `gantz/pattern`.
pub fn modules() -> &'static [SteelModule] {
    const MODULES: &[SteelModule] = &[gantz_rng::MODULE, MODULE];
    MODULES
}

/// The domain's base `.gantz` source. Named graphs wrapping the module
/// fns as thin expr nodes, plus a tick-driven demo.
pub const BASE_BYTES: &[u8] = include_bytes!("../base.gantz");

/// Earlier names of the base graphs, each as `(old, new)`. A host renames
/// stored graphs and references that still use an old name.
pub const BASE_RENAMES: &[(&str, &str)] = &[
    ("demo-pattern", "demo.pat"),
    ("demo-pplot", "demo.pat.plot"),
    ("papp", "pat.app"),
    ("pdegradeby", "pat.degradeby"),
    ("peuclid", "pat.euclid"),
    ("peuclidfull", "pat.euclidfull"),
    ("peuclidoff", "pat.euclidoff"),
    ("pfast", "pat.fast"),
    ("pfastcat", "pat.fastcat"),
    ("pfilter", "pat.filter"),
    ("pindices", "pat.indices"),
    ("pjoin", "pat.join"),
    ("pmap", "pat.map"),
    ("pmapevents", "pat.mapevents"),
    ("pmergewith", "pat.mergewith"),
    ("ppure", "pat.pure"),
    ("pquery", "pat.query"),
    ("prand", "pat.rand"),
    ("psaw", "pat.saw"),
    ("psched", "pat.sched"),
    ("pshift", "pat.shift"),
    ("psilence", "pat.silence"),
    ("pslow", "pat.slow"),
    ("pslowcat", "pat.slowcat"),
    ("pstack", "pat.stack"),
    ("psteady", "pat.steady"),
    ("ptimecat", "pat.timecat"),
    ("pwindow", "pat.window"),
];
