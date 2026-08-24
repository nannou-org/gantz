//! A tidalcycles-inspired pattern domain for gantz.
//!
//! A pattern is a Steel closure from a span (a pair of rational time
//! points) to the list of events occurring along it. Starting from simple
//! constructors, higher-order composition builds intricate patterns from
//! simple parts. The vocabulary lives in the [`MODULE`] Steel module so
//! patterns and their combinators compose as ordinary closures inside the
//! VM, and user functions (including graphs lifted via `fn-ref`) apply to
//! pattern values with no VM boundary to cross.
//!
//! The representation follows the `cycles` crate: events carry an `active`
//! span (where the value applies within the query) and an optional `whole`
//! span (the event's full structure, absent for continuous signals).

use gantz_core::vm::SteelModule;

/// The `gantz/pattern` Steel module.
///
/// Register via [`gantz_core::vm::new_engine`]/`init_with_modules` (or the
/// app's steel-module collection) and `(require "gantz/pattern")` to use.
/// All provided names carry the `pat/` prefix.
pub const MODULE: SteelModule = SteelModule {
    name: "gantz/pattern",
    src: include_str!("pattern.scm"),
};

/// The Steel modules provided by this domain.
pub fn modules() -> &'static [SteelModule] {
    const MODULES: &[SteelModule] = &[MODULE];
    MODULES
}
