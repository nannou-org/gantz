//! VM utilities for initializing and compiling gantz graphs.
//!
//! This module provides convenience wrappers around `gantz_core::vm` that
//! return the compiled module as a formatted string.

use gantz_core::Node;
use gantz_core::node::graph::Graph;
use gantz_core::vm::CompileError;
use steel::steel_vm::engine::Engine;

/// Initialize a new VM with root state and register the given graph.
///
/// Returns the initialized VM and the compiled module as a formatted string.
pub fn init<Env, N>(env: &Env, graph: &Graph<N>) -> Result<(Engine, String), CompileError>
where
    N: Node<Env>,
{
    let (vm, module) = gantz_core::vm::init(env, graph)?;
    Ok((vm, gantz_core::vm::fmt_module(&module)))
}

/// Compile the graph into a Steel module and run it in the VM.
///
/// Returns the compiled module as a formatted string.
pub fn compile<Env, N>(env: &Env, graph: &Graph<N>, vm: &mut Engine) -> Result<String, CompileError>
where
    N: Node<Env>,
{
    let module = gantz_core::vm::compile(env, graph, vm)?;
    Ok(gantz_core::vm::fmt_module(&module))
}
