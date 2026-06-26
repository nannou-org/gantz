//! VM utilities for initializing and compiling gantz graphs.
//!
//! This module provides:
//! - Convenience wrappers around `gantz_core::vm` (`init`, `compile`)
//! - Evaluation events and observer (`EvalEntryEvent`, `on_eval_entry`)
//! - The input-addressed VM synchronisation system ([`sync`])

use crate::BuiltinNodes;
use crate::head;
use crate::reg::{Registry, lookup_node};
use bevy_ecs::prelude::*;
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::node::{self, GetNode, graph::Graph};
use gantz_core::vm::{CompileError, Compiled};
use gantz_core::{Node, compile as core_compile, diagnostic};
use std::time::Duration;
use steel::steel_vm::engine::Engine;

/// The component updates for one compile attempt: the module/error outcome
/// and the extracted compile diagnostics.
fn compile_components(result: Result<Compiled, CompileError>) -> (head::Module, head::Diagnostics) {
    match result {
        Ok(module) => (
            head::Module {
                compiled: Some(module),
                error: None,
            },
            head::Diagnostics(vec![]),
        ),
        Err(e) => {
            let error = gantz_core::vm::error_chain(&e);
            log::error!("Failed to compile graph: {error}");
            let diags = diagnostic::from_compile_error(&e);
            let module = head::Module {
                compiled: e.into_module(),
                error: Some(error),
            };
            (module, head::Diagnostics(diags))
        }
    }
}

/// A function that produces entrypoints for a given graph.
pub type EntrypointFn<N> = Box<
    dyn for<'a> Fn(node::GetNode<'a>, &Graph<N>) -> Vec<core_compile::Entrypoint> + Send + Sync,
>;

/// Resource holding all entrypoint provider functions.
///
/// Each provider is called during compilation to collect entrypoints.
/// `GantzPlugin` registers `push_pull_entrypoints` by default.
/// Downstream plugins (e.g. `GantzEguiPlugin`) push additional providers.
#[derive(Resource)]
pub struct EntrypointFns<N: 'static>(pub Vec<EntrypointFn<N>>);

impl<N: 'static> Default for EntrypointFns<N> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

/// Collect all entrypoints by calling each provider fn in the resource.
fn collect_entrypoints<N: Node>(
    ep_fns: &EntrypointFns<N>,
    get_node: GetNode<'_>,
    graph: &Graph<N>,
) -> Vec<core_compile::Entrypoint> {
    ep_fns.0.iter().flat_map(|f| f(get_node, graph)).collect()
}

/// Resource holding the [`core_compile::Config`] used whenever a head's graph
/// is (re)compiled into its VM.
///
/// Defaults to the core defaults. Override (and trigger a recompile) to e.g.
/// enable `emit_all_node_fns` when debugging codegen in the module view.
#[derive(Default, Resource)]
pub struct CompileConfig(pub core_compile::Config);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The inputs that determine a head's compiled module.
#[derive(Clone, Copy, PartialEq)]
struct Inputs {
    /// The content address of the head's working graph.
    graph: ca::GraphAddr,
    /// The codegen configuration.
    config: core_compile::Config,
}

/// The inputs of a head's last compile *attempt* (success or failure).
///
/// `None` = never attempted. [`sync`] compares this against the current inputs
/// (the head's committed graph CA + config) to decide when to (re)compile -
/// there is no dirty flag to set or forget.
#[derive(Component, Default)]
pub struct CompiledInputs(Option<Inputs>);

/// When `true`, [`validate_committed`] hashes every open head's working graph
/// each frame and warns if it differs from the head's committed graph CA - i.e.
/// a system mutated the working graph without committing it, violating the
/// [`WorkingGraph`](head::WorkingGraph) commit-before-return invariant.
///
/// Defaults to `false` (no extra hashing); enable at runtime to debug a
/// suspected missing commit.
#[derive(Default, Resource)]
pub struct ValidateCommitted(pub bool);

/// Event to trigger evaluation of an entrypoint.
#[derive(Event)]
pub struct EvalEntryEvent {
    /// The head entity to evaluate on.
    pub head: Entity,
    /// The entrypoint to evaluate.
    pub entrypoint: core_compile::Entrypoint,
}

/// Emitted after VM evaluation completes, for timing capture.
///
/// This event allows UI layers (like `bevy_gantz_egui`) to observe VM execution
/// timing without the core crate depending on UI-related types.
#[derive(Event)]
pub struct EvalEntryComplete {
    /// The head entity that was evaluated.
    pub entity: Entity,
    /// The duration of the VM execution.
    pub duration: Duration,
}

// ---------------------------------------------------------------------------
// Core VM utilities
// ---------------------------------------------------------------------------

/// Initialize a new VM with root state and register the given graph.
///
/// Returns the initialized VM and the compiled module.
pub fn init<N>(
    get_node: GetNode,
    graph: &Graph<N>,
    entrypoints: &[core_compile::Entrypoint],
    config: &core_compile::Config,
) -> Result<(Engine, Compiled), CompileError>
where
    N: Node,
{
    gantz_core::vm::init(get_node, graph, entrypoints, config)
}

/// Compile the graph into a Steel module and run it in the VM.
pub fn compile<N>(
    get_node: GetNode,
    graph: &Graph<N>,
    vm: &mut Engine,
    entrypoints: &[core_compile::Entrypoint],
    config: &core_compile::Config,
) -> Result<Compiled, CompileError>
where
    N: Node,
{
    gantz_core::vm::compile(get_node, graph, vm, entrypoints, config)
}

// ---------------------------------------------------------------------------
// Observers
// ---------------------------------------------------------------------------

/// Observer that handles evaluation events by calling the appropriate VM function.
///
/// Emits an `EvalEntryComplete` event with timing information for UI layers to observe.
pub fn on_eval_entry(
    trigger: On<EvalEntryEvent>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<(&head::Module, &mut head::Diagnostics)>,
) {
    let event = trigger.event();
    let fn_name = core_compile::entry_fn_name(&event.entrypoint.id());
    if let Some(vm) = vms.get_mut(&event.head) {
        let start = web_time::Instant::now();
        let result = vm.call_function_by_name_with_args(&fn_name, vec![]);
        // Runtime diagnostics reflect the latest evaluation only.
        if let Ok((module, mut diagnostics)) = heads.get_mut(event.head) {
            diagnostics
                .0
                .retain(|d| d.severity != diagnostic::Severity::Runtime);
            if let (Err(e), Some(compiled)) = (&result, &module.compiled) {
                diagnostics
                    .0
                    .push(diagnostic::from_eval_error(e, vm, compiled));
            }
        }
        if let Err(e) = result {
            log::error!("{e}");
        }
        cmds.trigger(EvalEntryComplete {
            entity: event.head,
            duration: start.elapsed(),
        });
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Commit a head's working graph to the registry when it has diverged from the
/// head's current commit, updating the head and emitting a
/// [`head::CommittedEvent`]. Returns `true` if a new commit was made.
///
/// **Call this from any system that mutates a head's
/// [`WorkingGraph`](head::WorkingGraph), before the system returns** - it is how
/// the commit-before-return invariant (see `WorkingGraph`) is upheld, which in
/// turn lets [`sync`] recompile from the committed address without re-hashing.
/// This is the single place a working graph is content-addressed.
pub fn commit_working_graph<N>(
    registry: &mut Registry<N>,
    cmds: &mut Commands,
    entity: Entity,
    head: &mut ca::Head,
    graph: &Graph<N>,
) -> bool
where
    N: Clone + ca::CaHash,
{
    let graph_ca = ca::graph_addr(graph);
    let Some(head_commit) = registry.head_commit(head) else {
        return false;
    };
    if head_commit.graph == graph_ca {
        return false;
    }
    let old_head = head.clone();
    let new_commit_ca = registry.commit_graph_to_head(
        crate::reg::timestamp(),
        graph_ca,
        || crate::clone_graph(graph),
        head,
    );
    log::debug!("Graph changed -> {}", new_commit_ca.display_short());
    cmds.trigger(head::CommittedEvent {
        entity,
        old_head,
        new_head: head.clone(),
    });
    true
}

/// Keep every open head's VM in sync with the inputs to compilation.
///
/// The inputs are the head's *committed* graph content address and the
/// [`CompileConfig`]; each head's [`CompiledInputs`] memoizes the inputs of its
/// last compile attempt, and the VM is rebuilt whenever they differ. The
/// committed CA is read straight from the registry - **no per-frame hashing**
/// (#159): the [`WorkingGraph`](head::WorkingGraph) commit-before-return
/// invariant guarantees the working graph already matches it, and every change
/// is reflected either by a new commit (edits, via [`commit_working_graph`]) or
/// by a reset `CompiledInputs` (head open/replace/branch-move/resync), so
/// comparing committed CA + config is sufficient to drive recompiles.
///
/// Whether the rebuild is a fresh `init` or an in-place `compile` is decided by
/// VM presence in [`head::HeadVms`]: absent means a fresh init (head
/// replace/branch-move remove the VM to discard the old graph's node state);
/// present means an in-place compile, preserving node state (graph edits and
/// config changes).
pub fn sync<N>(
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    ep_fns: Res<EntrypointFns<N>>,
    config: Res<CompileConfig>,
    mut vms: NonSendMut<head::HeadVms>,
    mut heads_query: Query<head::OpenHeadData<N>, With<head::OpenHead>>,
) where
    N: 'static + Node + Clone + ca::CaHash + Send + Sync,
{
    for mut data in heads_query.iter_mut() {
        // The committed graph CA - the working graph already matches it (the
        // `WorkingGraph` invariant), so there is nothing to hash here.
        let Some(graph_ca) = registry.head_commit(&data.head_ref.0).map(|c| c.graph) else {
            continue;
        };
        let inputs = Inputs {
            graph: graph_ca,
            config: config.0,
        };
        if data.compiled_inputs.0 == Some(inputs) {
            continue;
        }

        // Rebuild the VM. On an in-place compile error the VM is kept (its
        // previous module remains evaluable) and the error surfaces via the
        // module/diagnostics components; a failed init leaves no VM, so eval
        // systems (e.g. `drive_frame_bangs`, `on_eval_entry`) skip the head
        // rather than driving a stale graph.
        let graph: &Graph<N> = &*data.working_graph;
        let get_node = |ca: &ca::ContentAddr| lookup_node(&registry, &**builtins, ca);
        let entrypoints = collect_entrypoints(&ep_fns, &get_node, graph);
        let result = match vms.get_mut(&data.entity) {
            None => init(&get_node, graph, &entrypoints, &config.0).map(|(vm, module)| {
                vms.insert(data.entity, vm);
                module
            }),
            Some(vm) => {
                gantz_core::graph::register(&get_node, graph, &[], vm);
                compile(&get_node, graph, vm, &entrypoints, &config.0)
            }
        };
        let (module, diagnostics) = compile_components(result);
        *data.module = module;
        *data.diagnostics = diagnostics;
        data.compiled_inputs.0 = Some(inputs);
    }
}

/// Debug check for the [`WorkingGraph`](head::WorkingGraph) commit-before-return
/// invariant.
///
/// When [`ValidateCommitted`] is enabled, hash every open head's working
/// graph and warn if it differs from the head's committed graph CA - i.e. a
/// system mutated the working graph without committing it. A no-op (no hashing)
/// when disabled, which is the default.
pub fn validate_committed<N>(
    validate: Res<ValidateCommitted>,
    registry: Res<Registry<N>>,
    heads: Query<head::OpenHeadDataReadOnly<N>, With<head::OpenHead>>,
) where
    N: 'static + ca::CaHash + Send + Sync,
{
    if !validate.0 {
        return;
    }
    for data in heads.iter() {
        let working = ca::graph_addr(&data.working_graph.0);
        let committed = registry.head_commit(&data.head_ref.0).map(|c| c.graph);
        if committed != Some(working) {
            log::warn!(
                "WorkingGraph invariant violated: head {:?} working graph ({}) does \
                 not match its committed graph ({:?}) - a system mutated it without \
                 committing (see `commit_working_graph`)",
                data.entity,
                working.display_short(),
                committed.map(|c| c.display_short().to_string()),
            );
        }
    }
}
