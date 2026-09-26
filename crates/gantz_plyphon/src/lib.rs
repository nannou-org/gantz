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
    BufferAccess, BufferBinding, BufferSource, DspBuilder, FADE_LAG, FadeSink, Finished, GainRef,
    NodeDsp, NodeRate, ParamBinding, PortShape, PortShapes, ScopeOutBinding, Signal, ToNodeDsp,
    node_dsp_of, signal_rate,
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
pub use port_info::{RootPortInfo, root_port_info};
pub use ref_ext::{DSP_REF_EXT_KEY, DspRefExt, dsp_graphs, is_dsp_graph};
pub use sugar::PlyphonSugar;
pub use units::{Emit, In, RateScale, UNITS, UnitDesc, UnitRate, unit_desc, unit_desc_by_keyword};
// `self::` disambiguates from the extern `egui` crate at the crate root.
#[cfg(feature = "egui")]
pub use self::egui::{
    DSP_PANE_KEY, DspEdgeStyle, DspPane, DspPaneHead, DspRefExtUi, DspSettingsTab, LoadSample,
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

/// Raw bytes of the DSP domain's base `.gantz` export, embedded at compile
/// time. The `bevy_gantz_plyphon` plugin contributes it as a base source. It
/// is self-contained. Its graphs compose builtin nodes and never ref other
/// sources.
pub const BASE_BYTES: &[u8] = include_bytes!("../base.gantz");
