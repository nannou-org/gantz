//! The gantz_core integration tests. They build as one binary, so the
//! dependency graph links once. Add new test files as modules here.

mod config;
mod diagnostics;
mod fn_apply;
mod graph;
mod modules;
mod nested;
mod rust_fn;
mod source_map;
mod state;
mod steel_target;
mod vm;
