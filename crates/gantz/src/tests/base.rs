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
