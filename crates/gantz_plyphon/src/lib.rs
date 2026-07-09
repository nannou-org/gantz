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
//! A DSP node's keyword and type mirror the underlying plyphon UGen it emits:
//! `~sinosc`/[`SinOsc`] -> `SinOsc`, `~scopeout`/[`ScopeOut`] -> `ScopeOut`,
//! `~out`/[`Out`] -> `Out`, `~lag`/[`Lag`] -> `Lag`. A node that composes *several*
//! UGens into one gantz node - or emits none, like the `~pack`/`~unpack`
//! channel-routing pair - gets its own descriptive name instead.

pub use backend::{AddAction, Backend, BackendError, Embedded, ROOT_GROUP_ID};
pub use builtin::builtins;
pub use compile::{
    BusBinding, DeriveError, Derived, RegionDerived, derive_synthdef, derive_synthdefs,
    structural_sig,
};
pub use dsp::{
    DspBuilder, FADE_LAG, GainRef, NodeDsp, NodeRate, ParamBinding, ScopeOutBinding, Signal,
    ToNodeDsp, node_dsp_of,
};
pub use flatten::{Flat, FlattenError, flatten, flatten_from_registry};
pub use node::{Bus, Lag, Out, Pack, ScopeOut, SinOsc, Unpack};
pub use sugar::PlyphonSugar;

pub mod backend;
pub mod builtin;
pub mod compile;
pub mod dsp;
pub mod flatten;
pub mod monitor;
pub mod node;
pub mod param;
pub mod sugar;
