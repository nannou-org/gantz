//! VM utilities for initializing and compiling gantz graphs.
//!
//! This module provides convenience wrappers around `gantz_core::vm` that
//! return the compiled module as a formatted string.

use gantz_core::Node;
use gantz_core::node::{GetNode, graph::Graph};
use gantz_core::vm::CompileError;
use steel::steel_vm::engine::Engine;

/// Initialize a new VM with root state and register the given graph.
///
/// Returns the initialized VM and the compiled module as a formatted string.
pub fn init<N>(get_node: GetNode, graph: &Graph<N>) -> Result<(Engine, String), CompileError>
where
    N: Node,
{
    let (vm, module) = gantz_core::vm::init(get_node, graph)?;
    Ok((vm, gantz_core::vm::fmt_module(&module)))
}

/// Compile the graph into a Steel module and run it in the VM.
///
/// Returns the compiled module as a formatted string.
pub fn compile<N>(
    get_node: GetNode,
    graph: &Graph<N>,
    vm: &mut Engine,
) -> Result<String, CompileError>
where
    N: Node,
{
    let module = gantz_core::vm::compile(get_node, graph, vm)?;
    Ok(gantz_core::vm::fmt_module(&module))
}
