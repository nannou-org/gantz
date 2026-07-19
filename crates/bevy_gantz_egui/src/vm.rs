//! The input-addressed VM synchronisation system ([`sync`]) and the
//! entrypoint provider collection ([`EntrypointFns`]), concrete over the
//! erased UI node ([`DynNode`]).

use crate::NodeCodecRes;
use crate::reg::{BuiltinNodes, GraphCache, lookup_node};
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_gantz::vm::{CompileConfig, Inputs};
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::node::{GetNode, graph::Graph};
use gantz_core::vm::{CompileError, Compiled};
use gantz_core::{Diagnostic, compile as core_compile, diagnostic};
use gantz_egui::node::DynNode;

/// A function that produces entrypoints for a given graph.
pub type EntrypointFn = Box<
    dyn for<'a> Fn(GetNode<'a>, &Graph<DynNode>) -> Vec<core_compile::Entrypoint> + Send + Sync,
>;

/// Resource holding all entrypoint provider functions.
///
/// Each provider is called during compilation to collect entrypoints.
/// `GantzEguiPlugin` registers `push_pull_entrypoints` plus the `update!` and
/// `tick!` providers. Downstream plugins may push additional providers.
///
/// Contribute via `get_resource_or_init` + push (never `insert_resource`,
/// which would clobber providers pushed by plugins built earlier) - the
/// convention every shared provider collection follows so that plugin order
/// does not matter.
#[derive(Default, Resource)]
pub struct EntrypointFns(pub Vec<EntrypointFn>);

/// Collect all entrypoints by calling each provider fn in the resource.
fn collect_entrypoints(
    ep_fns: &EntrypointFns,
    get_node: GetNode<'_>,
    graph: &Graph<DynNode>,
) -> Vec<core_compile::Entrypoint> {
    ep_fns.0.iter().flat_map(|f| f(get_node, graph)).collect()
}

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

/// Keep every open head's VM in sync with the inputs to compilation.
///
/// The inputs are the head's *committed* graph content address and the
/// [`CompileConfig`]; each head's
/// [`CompiledInputs`](bevy_gantz::CompiledInputs) memoizes the inputs of its
/// last compile attempt, and the VM is rebuilt whenever they differ. The
/// committed CA is read straight from the registry - **no per-frame hashing**
/// (#159): the [`WorkingGraph`](head::WorkingGraph) commit-before-return
/// invariant guarantees the working graph already matches it, and every change
/// is reflected either by a new commit (edits, via
/// [`commit_working_graph`](bevy_gantz::commit_working_graph)) or by a reset
/// `CompiledInputs` (head open/replace/branch-move/resync), so comparing
/// committed CA + config is sufficient to drive recompiles.
///
/// Whether the rebuild is a fresh `init` or an in-place `compile` is decided by
/// VM presence in [`head::HeadVms`]: absent means a fresh init (head
/// replace/branch-move remove the VM to discard the old graph's node state);
/// present means an in-place compile, preserving node state (graph edits and
/// config changes).
pub fn sync(
    registry: Res<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    ep_fns: Res<EntrypointFns>,
    config: Res<CompileConfig>,
    mut vms: NonSendMut<head::HeadVms>,
    mut heads_query: Query<head::OpenHeadData, With<head::OpenHead>>,
) {
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

        // Compilation reads the reified cache at the committed address (the
        // `WorkingGraph` invariant guarantees the working graph equals it).
        // Ensure the committed graph and its transitive references first. A
        // reify failure (e.g. an unknown tag rendered as a placeholder) marks
        // the attempt in `CompiledInputs` - no per-frame retry - and surfaces
        // a compile diagnostic attributed to the failing node so the scene
        // glows its placeholder; any previous VM stays evaluable.
        let reify = |nd: &ca::NodeData| codec.0.reify_ui(nd).map(|inst| inst.node);
        if let Err(e) = cache.0.ensure_with(&registry, [graph_ca.into()], reify) {
            log::error!("cannot compile head: {e}");
            // Attribute the failing node when it lies in the head's own root
            // graph; a failure within a referenced graph flags the whole
            // scene.
            let path = if e.graph == graph_ca {
                vec![e.source.node_ix]
            } else {
                vec![]
            };
            let message = format!("cannot compile: {e}");
            data.module.error = Some(message.clone());
            *data.diagnostics = head::Diagnostics(vec![Diagnostic {
                path,
                inputs: vec![],
                outputs: vec![],
                span: None,
                message,
                severity: diagnostic::Severity::Compile,
            }]);
            data.compiled_inputs.0 = Some(inputs);
            continue;
        }
        let Some(graph) = cache.get(&graph_ca) else {
            // The committed graph is missing from the registry outright.
            log::error!("cannot compile head: committed graph missing from the registry");
            continue;
        };

        // Rebuild the VM. On an in-place compile error the VM is kept (its
        // previous module remains evaluable) and the error surfaces via the
        // module/diagnostics components; a failed init leaves no VM, so eval
        // systems (e.g. `drive_update_bangs`, `on_eval_entry`) skip the head
        // rather than driving a stale graph.
        let get_node = |ca: &ca::ContentAddr| lookup_node(&cache, &builtins.instances, ca);
        let entrypoints = collect_entrypoints(&ep_fns, &get_node, graph);
        let result = match vms.get_mut(&data.entity) {
            None => gantz_core::vm::init(&get_node, graph, &entrypoints, &config.0).map(
                |(vm, module)| {
                    vms.insert(data.entity, vm);
                    module
                },
            ),
            Some(vm) => {
                gantz_core::graph::register(&get_node, graph, &[], vm);
                gantz_core::vm::compile(&get_node, graph, vm, &entrypoints, &config.0)
            }
        };
        let (module, diagnostics) = compile_components(result);
        *data.module = module;
        *data.diagnostics = diagnostics;
        data.compiled_inputs.0 = Some(inputs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Providers pushed into `EntrypointFns` before `GantzEguiPlugin` is
    /// added must survive its build: plugins contribute to the shared
    /// collection, they do not insert over it, so plugin order does not
    /// matter.
    #[test]
    fn entrypoint_fns_survive_plugin_order() {
        let mut app = bevy_app::App::new();
        app.world_mut()
            .get_resource_or_init::<EntrypointFns>()
            .0
            .push(Box::new(|_, _| Vec::new()));
        app.add_plugins(crate::GantzEguiPlugin::default());
        let fns = app.world().resource::<EntrypointFns>();
        assert_eq!(
            fns.0.len(),
            4,
            "the pre-pushed provider and the plugin's three seeds must survive",
        );
    }
}
