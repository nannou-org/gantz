use crate::DEFAULT_LOG_FILTER;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// A log sink that the test reads back.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// Warnings show from every crate, but iroh's network reports show only their
// errors.
#[test]
fn the_default_filter_hides_routine_network_reports() {
    let sink = Sink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER))
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        tracing::warn!(target: "iroh::net_report::report", "mapping varies");
        tracing::error!(target: "iroh::net_report::reportgen", "report failed");
        tracing::warn!(target: "iroh::socket", "relay lost");
        tracing::info!(target: "iroh::socket", "relay found");
        tracing::info!(target: "gantz_cli::vault", "ticket issued");
    });
    let logs = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(!logs.contains("mapping varies"), "{logs}");
    assert!(logs.contains("report failed"), "{logs}");
    assert!(logs.contains("relay lost"), "{logs}");
    assert!(!logs.contains("relay found"), "{logs}");
    assert!(logs.contains("ticket issued"), "{logs}");
}
