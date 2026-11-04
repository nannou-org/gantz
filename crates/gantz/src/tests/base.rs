use crate::conf;
use gantz_cli::headless;

#[test]
fn fmt_check_passes_on_base_files() {
    let conf = conf();
    let sources = headless::base_sources(&conf);
    let output = gantz_cli::fmt(&conf, &sources, 0..sources.len(), true);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(output.writes.is_empty());
}

#[test]
fn check_passes_on_base_sources() {
    let conf = conf();
    let sources = headless::base_sources(&conf);
    let output = gantz_cli::check(&conf, &sources, 0..sources.len());
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(output.warnings.is_empty(), "{:?}", output.warnings);
}

/// No base graph keeps its node state, and no base file stores any.
#[test]
fn base_graphs_keep_no_state() {
    use gantz_core::node::state;
    let conf = conf();
    let sources = headless::base_sources(&conf);
    let ts = gantz_edit::base::BASE_TIMESTAMP;
    let registry = headless::load_sources(&sources, &[], ts, &conf.codec).registry;
    let kept: Vec<String> = registry
        .heads()
        .filter(|(name, _)| state::keeps(&registry, &gantz_ca::Head::Branch((*name).clone())))
        .map(|(name, _)| name.to_string())
        .collect();
    assert!(kept.is_empty(), "base graphs keep their state: {kept:?}");
    let stored = gantz_ca::section_iter::<state::Snapshots>(&registry).count();
    assert_eq!(stored, 0, "base files store node state");
}
