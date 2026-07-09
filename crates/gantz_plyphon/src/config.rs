//! The DSP domain's runtime configuration and status.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Editable DSP settings (the Settings -> DSP tab).
///
/// This is both the domain's runtime configuration and the settings-change
/// payload emitted by [`DspSettingsTab`](crate::DspSettingsTab). Runtime-only
/// for now - not persisted, so it resets to the defaults each session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// How far ahead of the dsp clock a timestamped control update is scheduled -
    /// the latency↔sample-accuracy trade-off (must exceed output latency + frame
    /// jitter). The dsp driver reads this in place of a fixed constant.
    pub sched_lead: Duration,
    /// Whether DSP output is enabled. When `false`, the output stream is paused
    /// (muted) and resumed when re-enabled.
    pub enabled: bool,
}

/// Read-only DSP status, written by the DSP runtime for the GUI to display.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// Whether a DSP output device is present (else the app runs silent).
    pub present: bool,
    /// The active output device's name, if present.
    pub device: Option<String>,
    /// The output sample rate (Hz).
    pub sample_rate: f64,
    /// The number of output channels.
    pub channels: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sched_lead: Duration::from_millis(50),
            enabled: true,
        }
    }
}
