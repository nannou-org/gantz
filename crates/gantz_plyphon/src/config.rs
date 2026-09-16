//! The DSP domain's runtime configuration and status.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Editable DSP settings, the DSP tab of the Settings window.
///
/// This is both the domain's runtime configuration and the settings-change
/// payload emitted by `DspSettingsTab` behind the `egui` feature. It is not
/// persisted, so it resets to the defaults each session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// How far ahead of the dsp clock a timestamped control update is
    /// scheduled. It trades latency for sample accuracy and must exceed
    /// output latency plus frame jitter. The dsp driver reads it in place of
    /// a fixed constant.
    pub sched_lead: Duration,
    /// Whether DSP output is enabled. When `false`, the output stream is
    /// paused and muted. It resumes when re-enabled.
    pub enabled: bool,
}

/// The outcome of a head's most recent DSP derivation, written by the DSP
/// runtime for the GUI to display. It is the per-head analogue of [`Status`].
///
/// The error variants carry the rendered error message rather than the error
/// value. That keeps the type cheap to clone and the GUI decoupled from the
/// error enums.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DeriveStatus {
    /// Nothing has been derived for the head yet, or no DSP engine is
    /// running.
    #[default]
    Pending,
    /// The graph has no dsp sink, so it is silent by design. This is not an
    /// error.
    Silent,
    /// Derived and running.
    Ok {
        /// The number of resolved parts, synths, the head derived to.
        parts: usize,
    },
    /// Flattening the head's nested graphs failed. Carries the rendered
    /// [`crate::FlattenError`].
    FlattenError(String),
    /// Deriving the synthdef template failed. Carries the rendered
    /// [`crate::DeriveError`].
    DeriveError(String),
}

/// Read-only DSP status, written by the DSP runtime for the GUI to display.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// Whether a DSP output device is present. Otherwise the app runs silent.
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
