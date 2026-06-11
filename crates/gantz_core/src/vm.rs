//! Shared VM utilities for initializing and compiling gantz graphs.
//!
//! This module provides common functionality for working with the Steel VM
//! that is shared between different gantz frontends (Bevy app, pure egui demo, etc.).

use crate::{Edge, Node, compile::ModuleError, node};
use petgraph::visit::{Data, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, Visitable};
use steel::{SteelErr, SteelVal, parser::ast::ExprKind, steel_vm::engine::Engine};

/// Errors that can occur during VM compilation.
#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    /// Error generating the Steel module from the graph.
    #[error("module generation failed")]
    Module(#[from] ModuleError),
    /// Error executing a Steel expression in the VM.
    #[error("expression evaluation failed")]
    Eval(#[from] SteelErr),
}

/// Initialize a new VM with root state and register the given graph.
///
/// Returns the initialized VM and the generated module expressions.
pub fn init<'a, G>(
    get_node: node::GetNode<'a>,
    graph: G,
    entrypoints: &[crate::compile::Entrypoint],
    config: &crate::compile::Config,
) -> Result<(Engine, Vec<ExprKind>), CompileError>
where
    G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable
        + Copy,
    G::NodeWeight: Node,
{
    let mut vm = Engine::new_base();
    vm.register_value(crate::ROOT_STATE, SteelVal::empty_hashmap());
    crate::graph::register(get_node, graph, &[], &mut vm);
    let module = compile(get_node, graph, &mut vm, entrypoints, config)?;
    Ok((vm, module))
}

/// Compile the graph into a Steel module and run it in the VM.
///
/// This generates the Steel expressions for the graph and executes them
/// in the provided VM. Returns the generated expressions.
pub fn compile<'a, G>(
    get_node: node::GetNode<'a>,
    graph: G,
    vm: &mut Engine,
    entrypoints: &[crate::compile::Entrypoint],
    config: &crate::compile::Config,
) -> Result<Vec<ExprKind>, CompileError>
where
    G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable
        + Copy,
    G::NodeWeight: Node,
{
    let module = crate::compile::module(get_node, graph, entrypoints, config)?;
    for expr in &module {
        vm.run(expr.to_pretty(80))?;
    }
    Ok(module)
}

/// Format a compiled module as a human-readable string.
///
/// Each expression is pretty-printed with a width of 80 characters
/// and separated by blank lines.
pub fn fmt_module(module: &[ExprKind]) -> String {
    module
        .iter()
        .map(|expr| expr.to_pretty(80))
        .collect::<Vec<String>>()
        .join("\n\n")
}

/// Format an error together with its full [`std::error::Error::source`] chain.
///
/// `Display` renders only the outermost message, so a wrapper like
/// [`CompileError`] -> [`crate::compile::ModuleError`] -> the underlying cause
/// otherwise hides what actually went wrong (e.g. a bare "module generation
/// failed"). This walks the `source()` chain and appends each level on its own
/// `caused by:` line.
pub fn error_chain(err: &dyn std::error::Error) -> String {
    use std::fmt::Write;
    let mut s = err.to_string();
    let mut source = err.source();
    while let Some(e) = source {
        write!(s, "\ncaused by: {e}").expect("writing to a String never fails");
        source = e.source();
    }
    s
}
