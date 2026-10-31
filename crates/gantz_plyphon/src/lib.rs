//! DSP nodes for gantz plus a compiler that derives [`plyphon`] synthdefs from
//! connected subgraphs of [`NodeDsp`] nodes.
//!
//! Two independent backends compile the same gantz graph. The control-rate
//! Steel VM in `gantz_core` compiles every node. The plyphon audio engine
//! compiles the [`NodeDsp`] nodes via [`derive_synthdef`]. DSP nodes are inert
//! in the Steel world, so their [`Node::expr`](gantz_core::Node::expr) is a
//! placeholder. An audio driver such as `bevy_gantz_plyphon` installs and runs
//! the derived synthdefs through a [`Backend`].
//!
//! # Naming convention
//!
//! A DSP node's keyword mirrors the plyphon UGen it emits. `~sinosc` emits
//! `SinOsc` and `~lpf` emits `LPF`. The bespoke [`ScopeOut`] and [`Out`] nodes
//! follow the same rule with `~scopeout` and `~out`. Plain unit wrappers share
//! the one [`UnitNode`] type, driven by the [`units`] descriptor table. A node
//! that composes several UGens, or emits none like the `~pack`/`~unpack` pair,
//! gets its own type and a descriptive name.

pub use asset::{
    AudioAsset, AudioBuffers, BUFFER_SECTION, DecodeError, WavError, add_audio_asset, audio_asset,
};
pub use backend::{AddAction, Backend, BackendError, Embedded, ROOT_GROUP_ID};
pub use builtin::builtins;
pub use compile::{
    BusBinding, DeriveError, Derived, RegionDerived, content_def_name, derive_synthdef,
    derive_synthdefs, structural_sig,
};
pub use config::{Config, DeriveStatus, Status};
pub use describe::describe_parts;
pub use dsp::{
    BufferAccess, BufferBinding, BufferSource, DspBuilder, DspNode, FADE_LAG, FadeSink, Finished,
    GainRef, NodeDsp, NodeRate, ParamBinding, PortShape, PortShapes, ScopeOutBinding, Signal,
    ToNodeDsp, node_dsp_of, signal_rate,
};
pub use envelope::{Envelope, Segment, Shape};
pub use flatten::{
    AsRefNode, Flat, FlattenError, RefKind, flatten, flatten_from_registry,
    flatten_instance_children,
};
pub use instance::{
    BusKey, DefCache, GraphTemplate, InstancePart, Part, ResolvedBus, ResolvedPart, TemplateBus,
    TemplateRegion, VariantKey, derive_template, instantiate,
};
pub use node::{
    Buffer, Bus, Envgen, InvalidUnitNode, Out, Pack, Sample, ScopeOut, Sum, UnitNode, Unpack,
};
pub use node::{node_def, node_defs};
pub use port_info::{RootPortInfo, root_port_info};
pub use ref_ext::{DSP_REF_EXT_KEY, DspRefExt, dsp_graphs, is_dsp_graph};
pub use sugar::PlyphonSugar;
pub use units::{Emit, In, RateScale, UNITS, UnitDesc, UnitRate, unit_desc, unit_desc_by_keyword};
// `self::` disambiguates from the extern `egui` crate at the crate root.
#[cfg(feature = "egui")]
pub use self::egui::{
    DSP_PANE_KEY, DspEdgeStyle, DspPane, DspPaneHead, DspRefExtUi, DspSettingsTab,
};

pub mod asset;
pub mod backend;
pub mod builtin;
pub mod compile;
pub mod config;
pub mod describe;
pub mod dsp;
#[cfg(feature = "egui")]
pub mod egui;
pub mod envelope;
pub mod flatten;
pub mod instance;
pub mod monitor;
pub mod node;
pub mod param;
pub mod port_info;
pub mod ref_ext;
pub mod sugar;
pub mod units;
mod view;

/// The importers of the DSP domain: WAV files into `~sample` nodes.
pub fn importers() -> Vec<gantz_nodedef::import::Importer> {
    vec![gantz_nodedef::import::Importer {
        name: node::sample::WAV_IMPORTER.into(),
        description: "WAV".into(),
        extensions: vec!["wav".into()],
        section: BUFFER_SECTION.into(),
        import: std::sync::Arc::new(node::sample::import_wav),
    }]
}

/// The value of the state at `path` in `vm`.
#[cfg(test)]
fn state_value(
    vm: &gantz_core::steel::steel_vm::engine::Engine,
    path: &[usize],
) -> Option<gantz_core::datum::Datum> {
    let state = gantz_core::node::state::extract_value(vm, path).ok()??;
    gantz_core::datum::of_steel(&state).ok()
}

/// Raw bytes of the DSP domain's base `.gantz` export, embedded at compile
/// time. The `bevy_gantz_plyphon` plugin contributes it as a base source. It
/// is self-contained. Its graphs compose builtin nodes and never ref other
/// sources.
pub const BASE_BYTES: &[u8] = include_bytes!("../base.gantz");

/// Earlier names of the base graphs, each as `(old, new)`. A host renames
/// stored graphs and references that still use an old name.
pub const BASE_RENAMES: &[(&str, &str)] = &[
    ("demo-freeverb", "demo.freeverb"),
    ("demo-kick", "demo.kick"),
    ("demo-looper", "demo.looper"),
    ("demo-pluck", "demo.pluck"),
    ("demo-ringmod", "demo.ringmod"),
    ("demo-samplehold", "demo.samplehold"),
    ("demo-sampler", "demo.sampler"),
    ("demo-sine", "demo.sine"),
    ("demo-waveshape", "demo.waveshape"),
    ("demo-wavetable", "demo.wavetable"),
];
