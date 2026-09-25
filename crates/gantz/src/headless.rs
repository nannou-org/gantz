//! Headless registry loading and compilation, shared by the CLI and tests.
//!
//! No window, no store, no Bevy `App`. Everything here is built from the
//! node set in [`crate::node`] and the plain functions of the gantz crates.

use gantz_egui::node::DynNode;

/// The registry's graphs reified through the app's node codec.
pub type Reified = gantz_core::data::ReifiedGraphs<DynNode>;
/// The composed builtin palette plus one reified instance per builtin.
pub type Builtins = (gantz_core::Builtins, gantz_egui::node::UiBuiltins);

/// Reify the whole registry column into a typed cache through the codec.
///
/// Returns the failures rather than asserting, so a caller can report them.
pub fn reify_all(
    reg: &gantz_ca::Registry,
    codec: &gantz_egui::node::NodeCodec,
) -> (Reified, Vec<gantz_core::data::EnsureError>) {
    let mut reified = Reified::new();
    let errs = reified.ensure_all_with(reg, |nd| codec.reify_ui(nd).map(|inst| inst.node));
    (reified, errs)
}

/// The composed builtin palette plus one reified instance per builtin.
///
/// A builtin that fails to reify is a node-set composition error, so this
/// fails loudly, as the app does at startup.
pub fn builtins_with_instances() -> Builtins {
    let builtins = crate::node::builtins();
    let (instances, errs) = gantz_egui::node::UiBuiltins::reify(&builtins, &crate::node::codec());
    assert!(errs.is_empty(), "builtins failed to reify: {errs:?}");
    (builtins, instances)
}

/// The [`gantz_egui::Env`] over the given borrowed parts.
pub fn env<'a>(
    registry: &'a gantz_ca::Registry,
    reified: &'a Reified,
    builtins: &'a Builtins,
    codec: &'a gantz_egui::node::NodeCodec,
) -> gantz_egui::Env<'a> {
    gantz_egui::Env {
        registry,
        builtins: &builtins.0,
        codec,
        graphs: reified,
        instances: &builtins.1,
    }
}

/// The typed graph at the given head's tip, if reified.
pub fn head_graph<'a>(
    reified: &'a Reified,
    reg: &gantz_ca::Registry,
    head: &gantz_ca::Head,
) -> Option<&'a gantz_core::node::graph::Graph<DynNode>> {
    reified.get(&reg.head_commit(head)?.graph)
}

/// Every entrypoint the app compiles for a graph: push and pull sources plus
/// the `update!` and `tick!` providers `GantzEguiPlugin` registers.
pub fn entrypoints(
    get_node: gantz_core::node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<DynNode>,
) -> Vec<gantz_core::compile::Entrypoint> {
    let mut eps = gantz_core::compile::push_pull_entrypoints(get_node, graph);
    eps.extend(bevy_gantz_egui::node::update_bang::entrypoints(
        get_node, graph,
    ));
    eps.extend(bevy_gantz_egui::node::tick_bang::entrypoints(
        get_node, graph,
    ));
    eps
}

/// Compile and initialise a VM for the graph exactly as the app does, with
/// every entrypoint provider and the app's steel modules.
pub fn init(
    get_node: gantz_core::node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<DynNode>,
) -> Result<(steel::steel_vm::engine::Engine, gantz_core::vm::Compiled), gantz_core::vm::CompileError>
{
    let entrypoints = entrypoints(get_node, graph);
    let config = gantz_core::compile::Config::default();
    gantz_core::vm::init_with_modules(
        get_node,
        graph,
        &entrypoints,
        &config,
        &crate::node::steel_modules(),
    )
}
