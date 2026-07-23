//! DSP nodes for gantz plus a compiler that derives [`plyphon`] synthdefs from
//! connected subgraphs of [`NodeDsp`] nodes.
//!
//! The same gantz graph is compiled by two independent backends: the existing
//! control-rate Steel VM (`gantz_core`), and - for nodes implementing
//! [`NodeDsp`] - the plyphon audio engine via [`derive_synthdef`]. DSP nodes are
//! inert in the Steel world (their [`Node::expr`](gantz_core::Node::expr) is a
//! placeholder). An audio driver (see `bevy_gantz_plyphon`) installs and runs
//! the derived synthdefs through a [`Backend`].
//!
//! # Naming convention
//!
//! A DSP node's keyword mirrors the underlying plyphon UGen it emits:
//! `~sinosc` -> `SinOsc`, `~lpf` -> `LPF`, `~scopeout`/[`ScopeOut`] ->
//! `ScopeOut`, `~out`/[`Out`] -> `Out`. Plain unit wrappers are all the one
//! [`UnitNode`] type driven by the [`units`] descriptor table. A node that
//! composes *several* UGens into one gantz node - or emits none, like the
//! `~pack`/`~unpack` channel-routing pair - gets its own bespoke type and
//! descriptive name instead.

pub use asset::{
    AudioAsset, AudioBuffers, BUFFER_SECTION, DecodeError, add_audio_asset, audio_asset,
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
    BufferBinding, DspBuilder, FADE_LAG, Finished, GainRef, NodeDsp, NodeRate, ParamBinding,
    PortShape, PortShapes, ScopeOutBinding, Signal, ToNodeDsp, node_dsp_of, signal_rate,
};
pub use flatten::{
    AsRefNode, Flat, FlattenError, RefKind, flatten, flatten_from_registry,
    flatten_instance_children,
};
pub use instance::{
    BusKey, DefCache, GraphTemplate, InstancePart, Part, ResolvedBus, ResolvedPart, TemplateBus,
    TemplateRegion, VariantKey, derive_template, instantiate,
};
pub use node::{Bus, InvalidUnitNode, Out, Pack, PlayBuf, ScopeOut, Sum, UnitNode, Unpack};
pub use port_info::{RootPortInfo, root_port_info};
pub use ref_ext::{DSP_REF_EXT_KEY, DspRefExt, dsp_graphs, is_dsp_graph};
pub use sugar::PlyphonSugar;
pub use units::{In, UNITS, UnitDesc, unit_desc, unit_desc_by_keyword};
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
pub mod flatten;
pub mod instance;
pub mod monitor;
pub mod node;
pub mod param;
pub mod port_info;
pub mod ref_ext;
pub mod sugar;
pub mod units;

/// Raw bytes of the DSP domain's baked-in base `.gantz` export, embedded at
/// compile time. Contributed as a base source by `bevy_gantz_plyphon`'s
/// plugin; self-contained (its graphs compose builtin nodes, not refs into
/// other sources).
pub const BASE_BYTES: &[u8] = include_bytes!("../base.gantz");
