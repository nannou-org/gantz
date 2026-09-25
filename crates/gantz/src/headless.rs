//! Headless registry loading and compilation, shared by the CLI and tests.
//!
//! No window, no store, no Bevy `App`. Everything here is built from the
//! node set in [`crate::node`] and the plain functions of the gantz crates.

use gantz_egui::export::ParseExportError;
use gantz_egui::node::DynNode;
use std::borrow::Cow;

/// The registry's graphs reified through the app's node codec.
pub type Reified = gantz_core::data::ReifiedGraphs<DynNode>;
/// The composed builtin palette plus one reified instance per builtin.
pub type Builtins = (gantz_core::Builtins, gantz_egui::node::UiBuiltins);

/// One `.gantz` text to load. `label` names it in diagnostics.
pub struct Source {
    pub label: String,
    pub bytes: Cow<'static, [u8]>,
}

/// The result of loading a set of sources to a fixpoint.
pub struct Loaded {
    /// Every successfully parsed source merged, in source order.
    pub registry: gantz_ca::Registry,
    /// Each source's own parse, index-aligned with the input.
    pub parsed: Vec<Result<gantz_ca::Registry, ParseExportError>>,
}

/// The embedded base sources every app loads at startup.
pub fn base_sources() -> Vec<Source> {
    [
        ("<base:gantz>", gantz_base::BYTES),
        ("<base:plyphon>", gantz_plyphon::BASE_BYTES),
        ("<base:pattern>", gantz_pattern::BASE_BYTES),
    ]
    .into_iter()
    .map(|(label, bytes)| Source {
        label: label.to_string(),
        bytes: Cow::Borrowed(bytes),
    })
    .collect()
}

/// Parse every source seeded with the names loaded so far, to a fixpoint.
///
/// A source may reference names another source defines, so a source whose
/// references do not resolve yet is deferred to the next round. Once a full
/// round makes no progress the deferred sources keep their missing
/// dependency error. Later sources shadow earlier names, as in the app.
pub fn load_sources(
    sources: &[Source],
    now: gantz_ca::Timestamp,
    codec: &gantz_egui::node::NodeCodec,
) -> Loaded {
    let mut names = gantz_egui::reg::Names::new();
    let mut registry = gantz_ca::Registry::default();
    let mut parsed: Vec<Option<Result<gantz_ca::Registry, ParseExportError>>> =
        sources.iter().map(|_| None).collect();
    let parse = |ix: usize, names: &gantz_egui::reg::Names, registry: &gantz_ca::Registry| {
        let seed = bevy_gantz_egui::base::seed_graph_addrs(names, registry);
        gantz_egui::export::parse_export_seeded_at(&sources[ix].bytes, now, &seed, codec)
    };
    let is_missing_dep = |e: &ParseExportError| {
        matches!(
            e,
            ParseExportError::Format(e)
                if matches!(e.kind, gantz_format::ErrorKind::MissingDependency(_))
        )
    };
    let mut pending: Vec<usize> = (0..sources.len()).collect();
    loop {
        let mut deferred = vec![];
        for ix in pending.iter().copied() {
            match parse(ix, &names, &registry) {
                Err(e) if is_missing_dep(&e) => deferred.push(ix),
                Err(e) => parsed[ix] = Some(Err(e)),
                Ok(reg) => {
                    names.extend(reg.heads().map(|(n, ca)| (n.clone(), ca)));
                    registry.merge(reg.clone());
                    parsed[ix] = Some(Ok(reg));
                }
            }
        }
        if deferred.is_empty() {
            break;
        }
        if deferred.len() == pending.len() {
            for ix in deferred {
                parsed[ix] = Some(parse(ix, &names, &registry));
            }
            break;
        }
        pending = deferred;
    }
    let parsed = parsed
        .into_iter()
        .map(|p| p.expect("every source is parsed or deferred to a final parse"))
        .collect();
    Loaded { registry, parsed }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_gantz_egui::base::BASE_TIMESTAMP;

    /// A domain-style source wrapping the core `add` graph.
    const WRAP_ADD: &str = "\
(graph wrap-add
  (a inlet) (b inlet) (out outlet)
  (add0 (ref add #:sync))
  (-> a (add0 0)) (-> b (add0 1)) (-> add0 out))";

    fn wrap_add() -> Source {
        Source {
            label: "wrap-add.gantz".to_string(),
            bytes: Cow::Borrowed(WRAP_ADD.as_bytes()),
        }
    }

    fn has_head(reg: &gantz_ca::Registry, name: &str) -> bool {
        reg.heads().any(|(n, _)| n.to_string() == name)
    }

    #[test]
    fn base_sources_load_to_fixpoint() {
        let sources = base_sources();
        let loaded = load_sources(&sources, BASE_TIMESTAMP, &crate::node::codec());
        for (source, parsed) in sources.iter().zip(&loaded.parsed) {
            assert!(
                parsed.is_ok(),
                "{}: {:?}",
                source.label,
                parsed.as_ref().err()
            );
        }
        assert!(has_head(&loaded.registry, "add"));
        assert!(has_head(&loaded.registry, "demo-sine"));
        assert!(has_head(&loaded.registry, "demo-pattern"));
        let (_, errs) = reify_all(&loaded.registry, &crate::node::codec());
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn unseeded_foreign_ref_is_missing_dependency() {
        let codec = crate::node::codec();
        let loaded = load_sources(&[wrap_add()], BASE_TIMESTAMP, &codec);
        match &loaded.parsed[0] {
            Err(ParseExportError::Format(e)) => assert!(
                matches!(&e.kind, gantz_format::ErrorKind::MissingDependency(n) if n == "add"),
                "unexpected error kind: {e:?}",
            ),
            other => panic!("must not resolve unseeded: {other:?}"),
        }
        assert!(!has_head(&loaded.registry, "wrap-add"));

        let mut sources = base_sources();
        sources.push(wrap_add());
        let loaded = load_sources(&sources, BASE_TIMESTAMP, &codec);
        assert!(loaded.parsed.iter().all(Result::is_ok));
        let (reified, errs) = reify_all(&loaded.registry, &codec);
        assert!(errs.is_empty(), "{errs:?}");
        let builtins = builtins_with_instances();
        let reg_env = env(&loaded.registry, &reified, &builtins, &codec);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_env.node(ca);
        let head = gantz_ca::Head::Branch("wrap-add".parse().expect("name"));
        let graph = head_graph(&reified, &loaded.registry, &head).expect("wrap-add graph");
        init(&get_node, graph).unwrap_or_else(|e| {
            panic!(
                "wrap-add failed to compile:\n{}",
                gantz_core::vm::error_chain(&e)
            )
        });
    }
}
