//! Bevy + plyphon DSP runtime for gantz.
//!
//! [`PlyphonPlugin`] owns the dsp engine: at startup it opens a cpal output
//! stream (its own audio thread, running [`plyphon::World::fill_at`] so the engine
//! clock is anchored to the host) and keeps the [`plyphon::Controller`] +
//! [`plyphon::Nrt`] handles. Each update, after [`bevy_gantz::VmSet`], it derives
//! one synthdef per *part* of each open head's DSP subgraph (rooted at its
//! `~out` outputs and `~scopeout` monitors): the `~bus`-and-stage-cut regions,
//! plus one spawn per *instanced* nested-graph ref of its child's shared,
//! content-named defs (installed once, spawned per instance - see
//! [`gantz_plyphon::instance`]). Parts reconcile with the running synths via
//! the [`gantz_plyphon::Backend`] seam whenever the head's committed graph
//! changes: unchanged parts keep their synths (and their unit state -
//! oscillator phase, delay lines), changed ones are crossfade-replaced.
//!
//! The bridge runs both ways: control values drive dsp params via `set_control`,
//! and each `~scopeout` monitor's scope stream is drained here into the node's
//! ring-buffer state (so the control world can scope a dsp signal).
//!
//! Mixing across heads is free: every head's `~out` synth writes to output bus 0,
//! and plyphon sums all synths on that bus.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bevy_app::{App, Plugin, PreUpdate, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::system::NonSendMut;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use gantz_ca as ca;
use gantz_core::node::graph::Graph;
use gantz_plyphon::{
    AddAction, Backend, BusKey, DefCache, Embedded, GainRef, ROOT_GROUP_ID, ResolvedPart,
    ToNodeDsp, derive_template, flatten_from_registry, flatten_instance_children, instantiate,
};
use plyphon::{Controller, Nrt, Options, StreamConsumer, World, engine};
// `std::time::Instant` panics ("time not implemented") on `wasm32-unknown-unknown`;
// `web_time::Instant` shims it to `performance.now()` there and is plain `std::Instant`
// on native. This clock drives the crossfade fade-out deadlines and the bus-run graveyard.
use web_time::Instant;

/// Re-export of [`plyphon`] so downstream crates can implement custom units
/// (`Unit`, `UnitDef`, `unit_spec`, …) against the exact version this runtime
/// uses, without pinning it separately. See [`PlyphonPlugin::with_units`].
pub use plyphon;

/// A callback that registers custom plyphon units into the embedded engine's
/// [`plyphon::UnitRegistry`] at startup (see [`PlyphonPlugin::with_units`]).
type UnitRegistrar = Box<dyn Fn(&mut plyphon::UnitRegistry) + Send + Sync>;

use bevy_gantz::head::{HeadRef, HeadVms, OpenHead};
use bevy_gantz::{EntrypointSet, EvalEpoch, Registry, VmSet};
use bevy_gantz_egui::GraphCache;
use bevy_gantz_egui::{EdgeStyles, ExtPanes, RefExtUis, RegisterResponseExt, SettingsTabs};
use gantz_plyphon::{
    Config, DeriveStatus, DspEdgeStyle, DspPane, DspPaneHead, DspRefExtUi, DspSettingsTab,
    RootPortInfo, Status, describe_parts, root_port_info,
};

/// Editable DSP settings: the domain's [`Config`] as a bevy resource.
/// Runtime-only - not persisted, so it resets to the defaults each session.
#[derive(Clone, Debug, Default, Resource)]
pub struct DspConfig(pub Config);

/// Read-only DSP status: the domain's [`Status`] as a bevy resource, written
/// at startup for the Settings -> DSP tab to display.
#[derive(Clone, Debug, Default, Resource)]
pub struct DspStatus(pub Status);

/// A settings change emitted by the DSP settings tab (the full updated
/// [`Config`]), buffered for the settings-sync system to apply next frame.
#[derive(Message)]
pub struct DspSettingsChanged(pub Config);

/// Per-head DSP derive status plus a readable rendering of the derived
/// program - the DSP analogue of `bevy_gantz`'s `Module`/`Diagnostics` head
/// components. Written by the synth driver on each structural sync, i.e. only
/// when the head's committed graph changes. Heads the driver never reaches
/// (no dsp engine) carry no `DspHead`; readers treat a missing component as
/// [`DeriveStatus::Pending`].
#[derive(Clone, Component, Debug, Default)]
pub struct DspHead {
    /// The most recent derivation's outcome.
    pub status: DeriveStatus,
    /// The derived program rendered as text ([`describe_parts`]), or the
    /// failure message.
    pub view: std::sync::Arc<str>,
    /// The per-port shapes recorded at derive time, merged across the head's
    /// parts. Instanced parts arrive absolutized by their instance path (see
    /// [`gantz_plyphon::instance`]), so keys are node paths absolute to the
    /// head's root graph. Empty unless `status` is [`DeriveStatus::Ok`].
    pub shapes: std::sync::Arc<gantz_plyphon::PortShapes>,
}

/// OSC/NTP fixed-point units per second (OSC time is 32.32 fixed point: 2^32).
const OSC_UNITS_PER_SEC: f64 = 4_294_967_296.0;

/// The minimum time a crossfade-retired synth keeps running before its deferred
/// free (its fade gains ramp to zero over their own lags. A `LagControl` decays
/// to 0.1% within its lag, so this floor comfortably covers
/// [`gantz_plyphon::FADE_LAG`]). A longer gain lag extends the deadline to match.
const FADE_GRACE: Duration = Duration::from_millis(100);

/// The most crossfade-retired synths kept per head: a structural-weight drag
/// respawns every frame, so without a cap fades would pile up faster than they
/// expire. Freeing the oldest early is near-click-free - its gain has already
/// been decaying for at least a frame.
const MAX_FADING_PER_HEAD: usize = 2;

/// Frames per chunk in a `~scopeout`'s scope stream (one plyphon block).
const CHUNK_FRAMES: usize = 64;
/// Chunks pre-allocated per `~scopeout` scope stream (~43 ms of slack at 48 kHz before a
/// bounded overrun drops surplus - harmless, as the ring only keeps the last `size`).
const NUM_CHUNKS: usize = 32;

/// Convert monotonic [`EvalEpoch`] seconds to an absolute OSC/NTP engine-clock time.
fn osc(secs: f64) -> u64 {
    (secs * OSC_UNITS_PER_SEC) as u64
}

/// Plugin wiring plyphon DSP into a gantz bevy app.
///
/// Generic over `N`, the graph node type (must expose its DSP nodes via
/// [`ToNodeDsp`]).
///
/// To use custom UGens, register them with [`with_units`](Self::with_units). A
/// node's [`NodeDsp::ugens`](gantz_plyphon::NodeDsp::ugens) can then name them in a
/// [`UnitSpec`](plyphon::UnitSpec).
///
/// # The domain plugin shape
///
/// This plugin is the reference for wiring a domain into a gantz app: a
/// plain bevy `Plugin` (deliberately no `GantzDomain` umbrella trait or
/// plugin group - the plugin IS the domain's bevy-side assembly point),
/// addable in ANY order relative to the other gantz plugins:
///
/// - Cross-plugin resource reads (here the shared [`EvalEpoch`] clock)
///   happen in [`Plugin::finish`], which bevy runs after every plugin's
///   `build`, before the schedule first ticks.
/// - Contributions to shared collections go through
///   `get_resource_or_init` + push (see
///   [`bevy_gantz_egui::EntrypointFns`],
///   [`RegisterResponseExt::register_response_with`]) - never
///   `insert_resource`, which would clobber earlier contributions.
/// - GUI surfaces are provided by per-frame systems in `PreUpdate`
///   (here `sync_dsp_settings` and `provide_dsp_ref_ext`) pushing into the
///   `First`-cleared collections ([`SettingsTabs`], [`RefExtUis`],
///   [`EdgeStyles`]), whose `init_resource` calls are idempotent on purpose
///   so the plugin works with or without `GantzEguiPlugin`.
/// - The domain's own extension points (here
///   [`with_units`](Self::with_units)) hang off the plugin itself.
pub struct PlyphonPlugin {
    unit_registrars: Vec<UnitRegistrar>,
}

impl Default for PlyphonPlugin {
    fn default() -> Self {
        Self {
            unit_registrars: Vec::new(),
        }
    }
}

impl PlyphonPlugin {
    /// A plugin with no custom units (equivalent to [`default`](Self::default)).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register custom plyphon units into the embedded engine at startup, before
    /// any synth that names them is spawned. The callback gets the controller's
    /// [`plyphon::UnitRegistry`]. Call [`register`](plyphon::UnitRegistry::register)
    /// (or `register_demand`) once per unit. Chainable.
    ///
    /// ```ignore
    /// PlyphonPlugin::new().with_units(|reg| {
    ///     reg.register("Saw", Box::new(SawCtor));
    /// })
    /// ```
    pub fn with_units(
        mut self,
        f: impl Fn(&mut plyphon::UnitRegistry) + Send + Sync + 'static,
    ) -> Self {
        self.unit_registrars.push(Box::new(f));
        self
    }
}

impl Plugin for PlyphonPlugin {
    fn build(&self, app: &mut App) {
        // DSP settings (the Settings -> DSP tab).
        app.init_resource::<DspConfig>();
        // The Settings -> DSP tab: the tab's emitted `Config` payloads dispatch
        // into a buffered message, applied (and re-snapshotted into the tab)
        // by `sync_dsp_settings`. The `NamedRef` inspector's DSP `inline`
        // toggle is provided per frame by `provide_dsp_ref_ext`. The
        // `init_resource` calls are idempotent and keep the providers valid
        // without the egui plugin.
        app.init_resource::<SettingsTabs>()
            .init_resource::<ExtPanes>()
            .init_resource::<RefExtUis>()
            .init_resource::<EdgeStyles>()
            .add_message::<DspSettingsChanged>()
            .register_response_with::<Config>(dispatch_dsp_settings)
            .add_systems(
                PreUpdate,
                (
                    sync_dsp_settings,
                    provide_dsp_pane,
                    provide_dsp_ref_ext,
                    provide_dsp_edge_style,
                ),
            );
        // The DSP domain's base graphs (see `bevy_gantz_egui::base`).
        app.world_mut()
            .get_resource_or_init::<bevy_gantz_egui::base::BaseSources>()
            .0
            .push(bevy_gantz_egui::base::BaseSource {
                name: "plyphon",
                bytes: gantz_plyphon::BASE_BYTES,
            });
        // `.after(EntrypointSet)`: run once the `tick!`/`update!` drivers' triggered
        // evaluations have flushed, so the control values they queue are visible to
        // the param drain below in the same frame.
        app.add_systems(Update, drive_synths.after(VmSet).after(EntrypointSet));
    }

    // Engine construction lives in `finish` rather than `build`: it reads the
    // shared `EvalEpoch` clock, which another plugin (`GantzPlugin`) inserts
    // at build time. `finish` runs after every plugin's `build` and before
    // the schedule first ticks, so plugin order does not matter and the
    // systems registered above find their resources on the first frame.
    fn finish(&self, app: &mut App) {
        // The shared monotonic epoch is the dsp clock's time base. The cpal
        // callback anchors the engine clock to it via `fill_at`, matching
        // the firing times queued into `%args`.
        let epoch = *app.world().resource::<EvalEpoch>();
        let mut head_synths = HeadSynths::default();
        let status = match build_dsp_engine(epoch, &self.unit_registrars) {
            Some(engine) => {
                let status = Status {
                    present: true,
                    device: Some(engine.device.clone()),
                    sample_rate: engine.sample_rate,
                    channels: engine.out_channels,
                };
                head_synths.bus_alloc =
                    BusAlloc::new(engine.first_private_channel, engine.private_channels);
                app.insert_non_send(engine);
                status
            }
            None => {
                log::warn!("bevy_gantz_plyphon: no DSP output device; `~out` nodes will be silent");
                Status::default()
            }
        };
        app.insert_resource(DspStatus(status));
        // `HeadSynths` is a NonSend resource: it holds each `~scopeout`'s
        // scope `StreamConsumer` (a `!Sync` SPSC handle).
        app.insert_non_send(head_synths);
    }
}

/// The in-process plyphon engine: the control handle, the NRT cleanup handle, and
/// the held cpal output stream (kept alive for audio to continue). The plyphon
/// [`World`] itself lives inside the stream's audio callback.
struct DspEngine {
    controller: Controller,
    nrt: Nrt,
    out_channels: usize,
    /// The first *private* audio-bus channel (after the hardware output + input
    /// banks) and how many follow - the range [`BusAlloc`] hands out for `~bus`
    /// boundaries.
    first_private_channel: usize,
    private_channels: usize,
    /// The active output device's name (for the Settings -> DSP status readout).
    device: String,
    /// The output sample rate (Hz).
    sample_rate: f64,
    /// The cpal output stream; held to keep audio running, paused/played on mute.
    stream: cpal::Stream,
}

/// The audio-buffer blob entries: content address -> canonical encoded PCM
/// (the registry's `dsp.buffer` section entries).
type BufferBlobs = std::collections::BTreeMap<ca::ContentAddr, ca::Bytes>;

/// The empty buffer store, for registries with no `dsp.buffer` section.
static EMPTY_BUFFERS: BufferBlobs = BufferBlobs::new();

/// Per-head synth bookkeeping (which committed graph produced the currently
/// installed synthdef and running synth, so we only re-derive on change), plus the
/// allocator of global scope-stream indices for `~scopeout`s. A NonSend resource - a
/// `~scopeout`'s scope `StreamConsumer` is `Send` but not `Sync`.
#[derive(Default)]
struct HeadSynths {
    heads: HashMap<Entity, HeadParts>,
    scope_alloc: ScopeAlloc,
    bus_alloc: BusAlloc,
    /// Allocator of global buffer-table indices (bufnums) for resident assets.
    buffer_alloc: BufferAlloc,
    /// Assets currently installed in the engine's buffer table, keyed by
    /// address, shared read-only across every synth referencing them. Loaded
    /// once and refcounted; freed when the last reference retires.
    resident: HashMap<ca::ContentAddr, ResidentBuffer>,
    /// Installed synthdef names -> `(refcount, structural_sig)`. A def is
    /// installed once per name and reused while its structure is unchanged;
    /// a structural edit (same name, different sig) re-installs (retiring the
    /// old compiled def, which running synths keep via their own `Arc`). The
    /// refcount frees the def only when its last synth is retired. Per-head
    /// region defs (unique names per region) refcount 0 -> 1 -> 0 as before.
    shared_defs: HashMap<String, (usize, u64)>,
    /// Crossfade-retired synths still ramping their gains to zero, freed once
    /// their deadline passes (oldest first - entries are pushed in replacement
    /// order). Their scope streams ride along: a stream index must not be
    /// re-cued while the old `ScopeOut` (bufnum baked into its def) can still
    /// write it.
    fading: Vec<FadingSynth>,
    /// Memoised child templates, shared across heads and frames (content
    /// addressed, so entries stay valid for as long as their variant recurs).
    def_cache: DefCache,
}

/// One open head's running synths: one per resolved part, in part-DAG
/// topological order (bus writers before their readers - also their node-tree
/// order). Empty when the head's graph has no dsp sink.
struct HeadParts {
    graph: ca::GraphAddr,
    /// Re-run the structural sync next frame even though `graph` is current:
    /// a spawn failed transiently (command ring full). The ring drains within
    /// a block and the re-run is convergent - already-spawned parts match by
    /// key + sig + wiring and are kept.
    retry: bool,
    parts: Vec<PartSynth>,
}

/// A run of consecutive private audio-bus channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    start: usize,
    len: usize,
}

/// Allocates runs of consecutive private audio-bus channels for `~bus`
/// boundaries, keyed by (head, bus node path) so a bus keeps its channels
/// across re-derives (an unchanged region's def bakes its patched channel).
/// Released runs are quarantined until a deadline passes: a crossfade-retired
/// synth may still write them, and handing a run to another bus meanwhile would
/// sum unrelated audio into it.
#[derive(Default)]
struct BusAlloc {
    /// Free runs, sorted by start and coalesced.
    free: Vec<Run>,
    /// Live allocations.
    allocated: HashMap<(Entity, BusKey), Run>,
    /// Quarantined runs, returned to `free` once their deadline passes.
    graveyard: Vec<(Run, Instant)>,
}

impl BusAlloc {
    /// An allocator over the `count` private channels starting at `first`.
    fn new(first: usize, count: usize) -> Self {
        BusAlloc {
            free: vec![Run {
                start: first,
                len: count,
            }],
            ..Default::default()
        }
    }

    /// The run for `key`, `channels` wide - allocating, or re-allocating on a
    /// width change (the old run is quarantined). `None` = range exhausted.
    fn get_or_alloc(
        &mut self,
        key: (Entity, BusKey),
        channels: usize,
        now: Instant,
    ) -> Option<Run> {
        if let Some(&run) = self.allocated.get(&key) {
            if run.len == channels {
                return Some(run);
            }
            self.release(&key, now);
        }
        let ix = self.free.iter().position(|r| r.len >= channels)?;
        let run = Run {
            start: self.free[ix].start,
            len: channels,
        };
        if self.free[ix].len == channels {
            self.free.remove(ix);
        } else {
            self.free[ix].start += channels;
            self.free[ix].len -= channels;
        }
        self.allocated.insert(key, run);
        Some(run)
    }

    /// Quarantine `key`'s run (if any).
    fn release(&mut self, key: &(Entity, BusKey), now: Instant) {
        if let Some(run) = self.allocated.remove(key) {
            self.graveyard.push((run, now + FADE_GRACE * 2));
        }
    }

    /// Quarantine every run allocated to `entity`.
    fn release_head(&mut self, entity: Entity, now: Instant) {
        let keys: Vec<_> = self
            .allocated
            .keys()
            .filter(|(e, _)| *e == entity)
            .cloned()
            .collect();
        for key in keys {
            self.release(&key, now);
        }
    }

    /// Return quarantined runs whose deadline has passed to the free list.
    fn sweep(&mut self, now: Instant) {
        let mut ready = Vec::new();
        self.graveyard.retain(|(run, deadline)| {
            let due = *deadline <= now;
            if due {
                ready.push(*run);
            }
            !due
        });
        for run in ready {
            self.insert_free(run);
        }
    }

    /// Insert into the sorted free list, coalescing adjacent runs.
    fn insert_free(&mut self, run: Run) {
        let ix = self.free.partition_point(|r| r.start < run.start);
        self.free.insert(ix, run);
        if ix + 1 < self.free.len()
            && self.free[ix].start + self.free[ix].len == self.free[ix + 1].start
        {
            self.free[ix].len += self.free[ix + 1].len;
            self.free.remove(ix + 1);
        }
        if ix > 0 && self.free[ix - 1].start + self.free[ix - 1].len == self.free[ix].start {
            self.free[ix - 1].len += self.free[ix].len;
            self.free.remove(ix);
        }
    }
}

/// A synth retired by a crossfaded replacement: its gains have been set to zero
/// (ramping via their own lags) and it is freed once `deadline` passes.
struct FadingSynth {
    /// The head the synth belonged to (for the per-head backlog cap).
    entity: Entity,
    /// The running synth to free at `deadline`.
    node_id: i32,
    /// When the fade has died away and the synth (+ scopes) can be freed.
    deadline: Instant,
    /// The synth's cued scope streams, closed + freed only at `deadline`.
    scopes: Vec<ScopeSlot>,
    /// The assets whose refcount this synth still holds, released at `deadline`
    /// (not at fade-out) so the buffer stays resident through the crossfade.
    buffers: Vec<ca::ContentAddr>,
}

/// Allocates globally-unique scope-stream indices (plyphon recording-slot ids) for
/// live `~scopeout`s, reusing freed ones so a long editing session doesn't exhaust them.
#[derive(Default)]
struct ScopeAlloc {
    free: Vec<usize>,
    next: usize,
}

impl ScopeAlloc {
    /// A free index (reused if available, else a fresh one).
    fn alloc(&mut self) -> usize {
        self.free.pop().unwrap_or_else(|| {
            let index = self.next;
            self.next += 1;
            index
        })
    }

    /// Return an index for reuse.
    fn free(&mut self, index: usize) {
        self.free.push(index);
    }
}

/// How long a freed bufnum is quarantined before reuse: a just-retired
/// `PlayBuf`'s trailing blocks and its `buffer_free` command may still be in
/// flight when the index is handed out again, so a reuse must not `buffer_set`
/// over an index a fading reader can still see (mirrors the bus-run graveyard).
const BUFFER_GRACE: Duration = Duration::from_millis(200);

/// An asset installed in the engine's buffer table: the bufnum it occupies and
/// how many live-or-fading synths still reference it. Freed at refcount 0.
struct ResidentBuffer {
    bufnum: usize,
    refcount: usize,
}

/// Allocates global buffer-table indices (bufnums) for resident assets, reusing
/// freed ones. A freed index is quarantined for [`BUFFER_GRACE`] before it can
/// be reused, so a fading `PlayBuf` never reads a bufnum re-`buffer_set` under it.
#[derive(Default)]
struct BufferAlloc {
    free: Vec<usize>,
    next: usize,
    /// Freed indices awaiting the end of their grace before returning to `free`.
    graveyard: Vec<(usize, Instant)>,
}

impl BufferAlloc {
    /// A free bufnum (reused if available, else a fresh one).
    fn alloc(&mut self) -> usize {
        self.free.pop().unwrap_or_else(|| {
            let index = self.next;
            self.next += 1;
            index
        })
    }

    /// Quarantine `index` until `now + BUFFER_GRACE`, after which [`sweep`](Self::sweep)
    /// returns it to the free list.
    fn free(&mut self, index: usize, now: Instant) {
        self.graveyard.push((index, now + BUFFER_GRACE));
    }

    /// Return quarantined bufnums whose grace has passed to the free list.
    fn sweep(&mut self, now: Instant) {
        self.graveyard.retain(|(index, deadline)| {
            let due = *deadline <= now;
            if due {
                self.free.push(*index);
            }
            !due
        });
    }
}

/// The installed synthdef + running synth for one resolved part of a head
/// (a top-level region, or one instance's spawn of a shared child region).
struct PartSynth {
    /// The part's identity across re-derives (`ResolvedPart::key`), the
    /// driver's match for the keep/replace decision.
    key: u64,
    def_name: String,
    node_id: i32,
    /// Structural signature of the running synth's def (excludes param values) -
    /// unchanged across a structural-graph edit means the synth need not respawn.
    sig: u64,
    /// Hash of the part's bus wiring (keys + widths + params). A wiring change
    /// with an unchanged def (e.g. an inlet re-routed to another source)
    /// crossfade-respawns rather than live-switching the bus param - an abrupt
    /// bus switch clicks, while the respawn reuses the trusted fade machinery.
    wiring: u64,
    /// One slot per control param, binding a dsp node's state value to its synth
    /// param index, with the last value pushed via `set_control`.
    params: Vec<ParamSlot>,
    /// One slot per `~scopeout`: the cued scope stream whose samples the driver drains into
    /// the node's ring state each frame.
    scopes: Vec<ScopeSlot>,
    /// The def's driver-owned fade gains (one per sink), used to fade the synth
    /// in on spawn and out across a crossfaded replacement.
    gains: Vec<GainRef>,
    /// The distinct assets this synth references, each holding one refcount on
    /// its [`ResidentBuffer`]. Released (decrementing the refcount) only when
    /// the synth is finally freed - immediately if it has no fade, else at its
    /// [`FadingSynth`] deadline - so a crossfade respawn keeps the buffer
    /// resident with no reload flicker.
    buffers: Vec<ca::ContentAddr>,
}

/// Binds one synth control param to a dsp node's VM state value.
struct ParamSlot {
    /// The dsp node's path in the graph (where its value lives in VM state).
    node_path: Vec<usize>,
    /// The param's index within the synth.
    index: usize,
    /// The last value pushed via `set_control` (`None` until first applied).
    last: Option<f32>,
}

/// A `~scopeout`'s live scope stream: every sample of its dsp input arrives here (via
/// plyphon `ScopeOut` -> `cue_scope`) for the driver to append into the node's rings.
struct ScopeSlot {
    /// The `~scopeout` node's path in the graph (where its ring state lives).
    node_path: Vec<usize>,
    /// Each per-channel ring's length in *frames*.
    size: usize,
    /// The number of interleaved channels the stream carries (`cue_scope`'s width) -
    /// the tapped signal's width, inferred at derive time.
    channels: usize,
    /// The cued scope-stream index (a global recording-slot id, baked into the def's
    /// `ScopeOut` and freed on teardown).
    index: usize,
    /// The consumer the driver drains (`pop_filled`/`recycle`) each frame.
    consumer: StreamConsumer,
}

/// Dispatch a [`Config`] payload emitted by the DSP settings tab as a
/// buffered [`DspSettingsChanged`] message (registered via
/// [`RegisterResponseExt::register_response_with`]).
fn dispatch_dsp_settings(
    _entity: Option<Entity>,
    payload: gantz_egui::DynResponse,
    cmds: &mut Commands,
) {
    if let Ok(config) = payload.downcast::<Config>() {
        cmds.write_message(DspSettingsChanged(config));
    }
}

/// Apply any pending settings change, then provide this frame's DSP settings
/// tab: a fresh snapshot of the applied [`Config`] + [`Status`] (see
/// [`SettingsTabs`] for the First/PreUpdate schedule contract).
fn sync_dsp_settings(
    mut msgs: MessageReader<DspSettingsChanged>,
    mut config: ResMut<DspConfig>,
    status: Res<DspStatus>,
    tab_order: Res<bevy_gantz::head::HeadTabOrder>,
    heads: Query<(&HeadRef, Option<&DspHead>), With<OpenHead>>,
    mut tabs: ResMut<SettingsTabs>,
) {
    if let Some(msg) = msgs.read().last() {
        config.0 = msg.0.clone();
    }
    let heads = tab_order
        .iter()
        .filter_map(|&e| heads.get(e).ok())
        .map(|(head_ref, dsp)| {
            let status = dsp.map(|d| d.status.clone()).unwrap_or_default();
            (head_ref.0.to_string(), status)
        })
        .collect();
    tabs.0.push(Box::new(DspSettingsTab {
        config: config.0.clone(),
        status: status.0.clone(),
        heads,
    }));
}

/// Provide this frame's DSP pane: each open head's [`DspHead`] snapshot plus
/// the engine's presence (see [`ExtPanes`] for the First/PreUpdate schedule
/// contract). Heads without a [`DspHead`] read as [`DeriveStatus::Pending`].
fn provide_dsp_pane(
    status: Res<DspStatus>,
    heads: Query<(&HeadRef, Option<&DspHead>), With<OpenHead>>,
    mut panes: ResMut<ExtPanes>,
) {
    let heads = heads
        .iter()
        .map(|(head_ref, dsp)| {
            let dsp = dsp.cloned().unwrap_or_default();
            let head = DspPaneHead {
                status: dsp.status,
                view: dsp.view,
            };
            (head_ref.0.clone(), head)
        })
        .collect();
    panes.0.push(Box::new(DspPane {
        present: status.0.present,
        heads,
    }));
}

/// Provide this frame's DSP `NamedRef` inspector extension: the `inline`
/// toggle for references to DSP graphs (see [`RefExtUis`] for the
/// First/PreUpdate schedule contract).
///
/// The DSP-graph set is a pure walk over the stored registry data (see
/// [`gantz_plyphon::dsp_graphs`]), recomputed only when the registry changes
/// and handed to the UI.
fn provide_dsp_ref_ext(
    registry: Res<Registry>,
    mut dsp_graphs: Local<Option<std::sync::Arc<std::collections::HashSet<ca::ContentAddr>>>>,
    mut ref_ext_uis: ResMut<RefExtUis>,
) {
    if registry.is_changed() || dsp_graphs.is_none() {
        *dsp_graphs = Some(std::sync::Arc::new(gantz_plyphon::dsp_graphs(&registry)));
    }
    let dsp_graphs = dsp_graphs.clone().expect("just initialised");
    ref_ext_uis.0.push(Box::new(DspRefExtUi { dsp_graphs }));
}

/// Provide this frame's DSP edge styler: each open head's root-level port
/// classification, driving signal-edge rendering in the graph scene (see
/// [`EdgeStyles`] for the First/PreUpdate schedule contract).
///
/// Classification requires the concrete node type (see
/// [`root_port_info`]), so it is computed here and handed to the UI -
/// recomputed per head only when the registry, the head's working graph or
/// its [`DspHead`] change.
fn provide_dsp_edge_style(
    registry: Res<Registry>,
    reified: Res<GraphCache>,
    heads: Query<(&HeadRef, Option<Ref<DspHead>>), With<OpenHead>>,
    mut cache: Local<HashMap<ca::Head, std::sync::Arc<RootPortInfo>>>,
    mut edge_styles: ResMut<EdgeStyles>,
) {
    let mut styled: HashMap<ca::Head, std::sync::Arc<RootPortInfo>> = HashMap::new();
    for (head_ref, dsp) in heads.iter() {
        let head = head_ref.0.clone();
        // The head's committed graph, read from the reified cache (the
        // working graph equals it by the `WorkingGraph` invariant). An edit
        // commits, so `registry.is_changed()` covers working-graph changes.
        let Some(graph) = registry
            .head_commit(&head)
            .and_then(|c| reified.get(&c.graph))
        else {
            continue;
        };
        let stale = registry.is_changed()
            || reified.is_changed()
            || dsp.as_ref().is_some_and(|d| d.is_changed())
            || !cache.contains_key(&head);
        if stale {
            let shapes = dsp.as_ref().map(|d| d.shapes.clone()).unwrap_or_default();
            let info = root_port_info(graph, &reified.0, &shapes);
            cache.insert(head.clone(), std::sync::Arc::new(info));
        }
        styled.insert(head.clone(), cache[&head].clone());
    }
    // Forget heads that are no longer open.
    cache.retain(|head, _| styled.contains_key(head));
    edge_styles.0.push(Box::new(DspEdgeStyle { heads: styled }));
}

/// Keep each open head's synth in sync, `.after(VmSet)` and `.after(EntrypointSet)`.
/// Two passes per head:
///
/// - *Structural sync* (only when the head's committed graph changed): re-derive
///   the synthdef and respawn the synth if its structure changed (else keep it).
/// - *Param sync* (every frame): drain each dsp param's queued `(time, value)`
///   control updates from VM state and *schedule* them on the running synth at
///   `time + SCHED_LEAD` via [`Backend::set_control_at`], so automation plays
///   sample-accurately. A direct inspector edit (no queue) is applied immediately.
///   Either way the synth is not respawned (preserving phase), and value/automation
///   edits no longer change the graph address.
/// - *Scope sync* (every frame): drain each `~scopeout`'s scope stream and append its
///   samples into the node's ring state (capped at the tap's `size`).
///
/// Also tears down synths for closed heads.
fn drive_synths(
    registry: Res<Registry>,
    reified: Res<GraphCache>,
    dsp: Option<NonSendMut<DspEngine>>,
    dsp_config: Res<DspConfig>,
    mut enabled_applied: Local<Option<bool>>,
    state: NonSendMut<HeadSynths>,
    mut vms: NonSendMut<HeadVms>,
    heads: Query<(Entity, &HeadRef), With<OpenHead>>,
    mut cmds: Commands,
) {
    let Some(dsp) = dsp else {
        return;
    };
    let dsp = dsp.into_inner();
    let state = state.into_inner();

    // Apply the enable/mute toggle by pausing/playing the output stream on change.
    if *enabled_applied != Some(dsp_config.0.enabled) {
        let result = if dsp_config.0.enabled {
            dsp.stream.play()
        } else {
            dsp.stream.pause()
        };
        if let Err(e) = result {
            log::error!("bevy_gantz_plyphon: audio stream play/pause failed: {e}");
        }
        *enabled_applied = Some(dsp_config.0.enabled);
    }

    // Tick NRT cleanup off the audio thread (drops freed synths, surfaces events).
    dsp.nrt.process();
    while dsp.nrt.poll().is_some() {}
    // Drop retired compiled defs once the audio thread is done with them.
    // Frees without a follow-up install would otherwise linger in `retiring`.
    dsp.controller.reap_retired_defs();

    let out_channels = dsp.out_channels;
    let sample_rate = dsp.sample_rate;
    let mut live: HashSet<Entity> = HashSet::new();

    for (entity, head_ref) in heads.iter() {
        live.insert(entity);
        let Some(graph_ca) = registry.head_commit(&head_ref.0).map(|c| c.graph) else {
            continue;
        };
        // The head's committed graph, read from the reified cache (the
        // working graph equals it by the `WorkingGraph` invariant). A cache
        // miss (e.g. an unknown-tag reify failure) keeps the previous synths.
        let Some(graph) = reified.get(&graph_ca) else {
            continue;
        };

        // Structural sync: only when the committed graph changed. Flattening
        // first splices any nested graphs (resolved through the registry) into
        // one flat graph whose nodes carry their original nested paths. On a
        // flatten error (a ref cycle or an unresolvable ref, both forbidden
        // upstream): keep the previous synths, parking the head at this graph
        // address so the error logs once per commit rather than every frame.
        // Either way the outcome lands on the head as a `DspHead` for the GUI.
        let stale = state
            .heads
            .get(&entity)
            .is_none_or(|h| h.graph != graph_ca || h.retry);
        if stale {
            let flat_children = flatten_from_registry(graph, &reified).and_then(|flat| {
                let children = flatten_instance_children(&flat, &reified)?;
                Ok((flat, children))
            });
            let dsp_head = match flat_children {
                Ok((flat, children)) => structural_sync(
                    &mut dsp.controller,
                    state,
                    buffer_blobs(&registry),
                    entity,
                    graph_ca,
                    &flat,
                    &children,
                    out_channels,
                    sample_rate,
                ),
                Err(e) => {
                    log::error!(
                        "bevy_gantz_plyphon: flattening nested graphs failed ({e}), \
                         keeping the previous synths"
                    );
                    park_head(state, entity, graph_ca);
                    DspHead {
                        status: DeriveStatus::FlattenError(e.to_string()),
                        view: format!("{e} - keeping the previous synths").into(),
                        shapes: Default::default(),
                    }
                }
            };
            cmds.entity(entity).insert(dsp_head);
        }

        // Param sync: drain each param's queued control updates and schedule
        // them ahead of the dsp clock. direct (untimestamped) value edits apply
        // immediately.
        let (Some(head), Some(vm)) = (state.heads.get_mut(&entity), vms.get_mut(&entity)) else {
            continue;
        };
        for synth in &mut head.parts {
            let node_id = synth.node_id;
            let mut backend = Embedded::new(&mut dsp.controller);
            for slot in &mut synth.params {
                let Some((value, pending)) = gantz_plyphon::param::drain_param(vm, &slot.node_path)
                else {
                    continue;
                };
                let value = value as f32;
                if pending.is_empty() {
                    // No automation this frame: apply a direct inspector edit
                    // immediately, only when the value actually changed.
                    if slot.last.map(f32::to_bits) != Some(value.to_bits()) {
                        if let Err(e) = backend.set_control(node_id, slot.index, value) {
                            log::error!("bevy_gantz_plyphon: set_control failed: {e:?}");
                        }
                        slot.last = Some(value);
                    }
                } else {
                    // Timestamped automation: schedule each update at its own time
                    // plus the lead, preserving the inter-tick spacing.
                    for (t, v) in pending {
                        let when = osc(t + dsp_config.0.sched_lead.as_secs_f64());
                        if let Err(e) = backend.set_control_at(node_id, slot.index, v as f32, when)
                        {
                            log::error!("bevy_gantz_plyphon: set_control_at failed: {e:?}");
                        }
                    }
                    // The latest queued value is now current; record it so a later
                    // immediate pass doesn't resend it.
                    slot.last = Some(value);
                }
            }

            // Scope sync: drain each `~scopeout`'s scope stream and append every streamed
            // sample into its per-channel ring state (the list-of-rings its control
            // `expr` surfaces on a trigger push. `push_ring` deinterleaves and keeps
            // the last `size` frames per channel).
            for scope in &mut synth.scopes {
                let mut samples = Vec::new();
                while let Some(chunk) = scope.consumer.pop_filled() {
                    samples.extend_from_slice(chunk.filled_samples());
                    scope.consumer.recycle(chunk);
                }
                if !samples.is_empty() {
                    gantz_plyphon::monitor::push_ring(
                        vm,
                        &scope.node_path,
                        &samples,
                        scope.size,
                        scope.channels,
                    );
                }
            }
        }
    }

    // Fade out synths whose heads are no longer open (their defs are freed now -
    // safe, a running synth holds its own compiled def until freed).
    let stale: Vec<Entity> = state
        .heads
        .keys()
        .copied()
        .filter(|e| !live.contains(e))
        .collect();
    for e in stale {
        if let Some(head) = state.heads.remove(&e) {
            for synth in head.parts {
                let def_name = synth.def_name.clone();
                fade_out(&mut dsp.controller, state, e, synth);
                release_def(&mut dsp.controller, &mut state.shared_defs, &def_name);
            }
        }
        state.bus_alloc.release_head(e, Instant::now());
    }

    // Sweep crossfade-retired synths, freeing those whose fade has died away,
    // and return quarantined bus runs whose grace has passed.
    for f in expire_fades(&mut state.fading, Instant::now()) {
        let _ = Embedded::new(&mut dsp.controller).free_node(f.node_id);
        free_scopes(&mut dsp.controller, &mut state.scope_alloc, f.scopes);
        free_buffers(&mut dsp.controller, state, &f.buffers, Instant::now());
    }
    state.bus_alloc.sweep(Instant::now());
    state.buffer_alloc.sweep(Instant::now());
}

/// Split the fade backlog: drains and returns the entries due for freeing - those
/// past their deadline, plus the *oldest* entries of any head whose backlog
/// exceeds [`MAX_FADING_PER_HEAD`] (entries are pushed in replacement order, so a
/// structural-weight drag that respawns every frame retires its pile-up early.
/// Near-click-free, as their gains have already been decaying).
fn expire_fades(fading: &mut Vec<FadingSynth>, now: Instant) -> Vec<FadingSynth> {
    let mut backlog: HashMap<Entity, usize> = HashMap::new();
    for f in fading.iter() {
        *backlog.entry(f.entity).or_default() += 1;
    }
    let mut expired = Vec::new();
    for f in std::mem::take(fading) {
        let count = backlog.get_mut(&f.entity).expect("counted above");
        if f.deadline <= now || *count > MAX_FADING_PER_HEAD {
            *count -= 1;
            expired.push(f);
        } else {
            fading.push(f);
        }
    }
    expired
}

/// Re-derive `entity`'s per-region synthdefs and reconcile them with the running
/// synths: a region whose key + structural signature both match keeps its synth
/// untouched (its unit state - oscillator phase, delay lines - survives exactly).
/// A changed region is crossfade-replaced. A disappeared region fades out. Every
/// signature is computed on the final def - bus indices, `ScopeOut` bufnums
/// and fade defaults are all no-lag control params / baked defaults excluded
/// from [`structural_sig`] - so a re-derive of the same graph never spuriously
/// respawns (and the driver never mutates a def copy).
///
/// Park `entity` at `graph_ca` without touching its running synths, so an
/// unactionable commit (a flatten or derive error) is not retried (and its
/// error not re-logged) every frame.
fn park_head(state: &mut HeadSynths, entity: Entity, graph_ca: ca::GraphAddr) {
    match state.heads.get_mut(&entity) {
        Some(head) => head.graph = graph_ca,
        None => {
            state.heads.insert(
                entity,
                HeadParts {
                    graph: graph_ca,
                    retry: false,
                    parts: Vec::new(),
                },
            );
        }
    }
}

/// A replacement spawns *silent* (fade defaults patched to `0.0`. Defaults seed
/// both the control wire and the lag state) and ramps its fades to unity once
/// up, while the old synth's fades ramp to zero ahead of a deferred free - the
/// overlap is the crossfade (on a bus, `Out` sums the two ramps). Placement
/// follows the region DAG: a spawned synth lands `Before` the first kept synth
/// later in topo order (bus readers hear only writers computed earlier in the
/// node tree), else at the tail. On install/spawn failure the old synth is left
/// playing - strictly better than going silent.
///
/// The registry's audio-buffer blob entries, or the empty store when the
/// `dsp.buffer` section is absent.
fn buffer_blobs(reg: &ca::Registry) -> &BufferBlobs {
    reg.blobs()
        .get(gantz_plyphon::BUFFER_SECTION)
        .map(|store| &store.entries)
        .unwrap_or(&EMPTY_BUFFERS)
}

/// Returns the sync's outcome as the head's [`DspHead`], for the GUI.
fn structural_sync<N>(
    controller: &mut Controller,
    state: &mut HeadSynths,
    assets: &BufferBlobs,
    entity: Entity,
    graph_ca: ca::GraphAddr,
    graph: &Graph<gantz_plyphon::Flat<N>>,
    children: &HashMap<gantz_ca::ContentAddr, Graph<gantz_plyphon::Flat<N>>>,
    out_channels: usize,
    sample_rate: f64,
) -> DspHead
where
    N: ToNodeDsp,
{
    let now = Instant::now();
    let derive_start = Instant::now();
    let resolve = |ca: &gantz_ca::ContentAddr| children.get(ca);
    let template_result = derive_template(graph, out_channels, &resolve, &mut state.def_cache);
    log::debug!("Derived synthdef template ({:?})", derive_start.elapsed());
    let derived: Vec<ResolvedPart> = match template_result {
        Ok(template) => instantiate(&template, &state.def_cache),
        // No dsp sink to root any synthdef at: fade every region out. Freeing
        // the defs now is safe - a fading synth holds its own compiled copy.
        Err(gantz_plyphon::DeriveError::NoSink) => {
            if let Some(head) = state.heads.remove(&entity) {
                for synth in head.parts {
                    let def_name = synth.def_name.clone();
                    fade_out(controller, state, entity, synth);
                    release_def(controller, &mut state.shared_defs, &def_name);
                }
            }
            state.bus_alloc.release_head(entity, now);
            // An empty entry parks the head at this graph address so sinkless
            // graphs don't re-derive every frame.
            state.heads.insert(
                entity,
                HeadParts {
                    graph: graph_ca,
                    retry: false,
                    parts: Vec::new(),
                },
            );
            return DspHead {
                status: DeriveStatus::Silent,
                view: "no dsp sink (`~out` / `~scopeout`) - silent".into(),
                shapes: Default::default(),
            };
        }
        // A part cycle (`~bus` regions or instances with no runnable
        // writer-before-reader order) or an instanced-reference failure: keep
        // the previous synths, parking the head so the error logs once per
        // commit rather than every frame.
        Err(e) => {
            log::error!("bevy_gantz_plyphon: {e}, keeping the previous synths");
            park_head(state, entity, graph_ca);
            return DspHead {
                status: DeriveStatus::DeriveError(e.to_string()),
                view: format!("{e} - keeping the previous synths").into(),
                shapes: Default::default(),
            };
        }
    };

    // The GUI's readable rendering and the merged per-port shapes, taken
    // before the planning loop below consumes `derived`.
    let view: std::sync::Arc<str> = describe_parts(&derived).into();
    let shapes: std::sync::Arc<gantz_plyphon::PortShapes> = std::sync::Arc::new(
        derived
            .iter()
            .flat_map(|r| r.shapes.iter().map(|(k, v)| (k.clone(), *v)))
            .collect(),
    );
    let n_parts = derived.len();

    // Release bus runs whose keys are gone from the derivation.
    let live_buses: HashSet<BusKey> = derived
        .iter()
        .flat_map(|r| r.bus_writes.iter().chain(&r.bus_reads))
        .map(|b| b.key.clone())
        .collect();
    let dead_buses: Vec<_> = state
        .bus_alloc
        .allocated
        .keys()
        .filter(|(e, key)| *e == entity && !live_buses.contains(key))
        .cloned()
        .collect();
    for key in dead_buses {
        state.bus_alloc.release(&key, now);
    }

    // Plan each part: keep the running synth when key + sig + wiring match,
    // else spawn a replacement (fading any predecessor once the replacement
    // is up).
    enum Plan {
        Keep(PartSynth),
        Spawn(ResolvedPart, u64, Option<PartSynth>),
    }
    let mut prev = state
        .heads
        .remove(&entity)
        .map(|h| h.parts)
        .unwrap_or_default();
    let mut plans: Vec<Plan> = Vec::with_capacity(derived.len());
    for r in derived {
        let wiring = wiring_hash(&r);
        match prev.iter().position(|p| p.key == r.key) {
            Some(ix) => {
                let mut p = prev.remove(ix);
                if p.sig == r.sig && p.wiring == wiring {
                    // Structure + wiring unchanged (e.g. a non-dsp edit, or a
                    // ring-size edit that is not in the def): keep the synth +
                    // its param slots + its cued scope streams. Refresh each
                    // tap's ring `size` in case a size edit changed it.
                    for m in &r.monitors {
                        if let Some(slot) = p.scopes.iter_mut().find(|s| s.node_path == m.node_path)
                        {
                            slot.size = m.size;
                            // A width change always changes the `ScopeOut` unit's
                            // input count and thus the sig - it can't reach here.
                            debug_assert_eq!(
                                slot.channels, m.channels,
                                "sig-unchanged sync must not change a scope's width",
                            );
                        }
                    }
                    plans.push(Plan::Keep(p));
                } else {
                    plans.push(Plan::Spawn(r, wiring, Some(p)));
                }
            }
            None => plans.push(Plan::Spawn(r, wiring, None)),
        }
    }
    // Parts that disappeared entirely: fade out + free their defs.
    for old in prev {
        let def_name = old.def_name.clone();
        fade_out(controller, state, entity, old);
        release_def(controller, &mut state.shared_defs, &def_name);
    }

    // Each spawn's node-tree anchor: the first KEPT synth later in topo order
    // (spawning `Before` it keeps writers ahead of readers. Successive spawns
    // before the same anchor preserve their relative order), else the tail.
    let anchors: Vec<Option<i32>> = (0..plans.len())
        .map(|i| {
            plans[i + 1..].iter().find_map(|p| match p {
                Plan::Keep(k) => Some(k.node_id),
                Plan::Spawn(..) => None,
            })
        })
        .collect();

    let mut parts: Vec<PartSynth> = Vec::with_capacity(plans.len());
    let mut transient_failure = false;
    for (plan, anchor) in plans.into_iter().zip(anchors) {
        match plan {
            Plan::Keep(k) => parts.push(k),
            Plan::Spawn(r, wiring, old) => {
                match spawn_part(
                    controller,
                    state,
                    assets,
                    entity,
                    r,
                    wiring,
                    sample_rate,
                    anchor,
                ) {
                    Ok(synth) => {
                        // The replacement is live: fade the old out (the gain
                        // overlap is the crossfade) and release its def
                        // refcount. If the new synth shares the name (stable
                        // key), the refcount was bumped by `spawn_part`, so
                        // this just drops the old's reference (the def stays
                        // installed for the new synth); if the name differs,
                        // the old def frees now (the old synth keeps its own
                        // `Arc` while it fades out).
                        if let Some(old) = old {
                            let old_def_name = old.def_name.clone();
                            fade_out(controller, state, entity, old);
                            release_def(controller, &mut state.shared_defs, &old_def_name);
                        }
                        parts.push(synth);
                    }
                    // Keep the old synth playing untouched on failure -
                    // strictly better than the silence a teardown would leave.
                    // A transient failure (full command ring) marks the head
                    // for a convergent re-sync next frame.
                    Err(e) => {
                        transient_failure |= matches!(e, SpawnError::Transient);
                        parts.extend(old);
                    }
                }
            }
        }
    }
    state.heads.insert(
        entity,
        HeadParts {
            graph: graph_ca,
            retry: transient_failure,
            parts,
        },
    );

    DspHead {
        status: DeriveStatus::Ok { parts: n_parts },
        view,
        shapes,
    }
}

/// Why a part failed to spawn.
#[derive(Debug)]
enum SpawnError {
    /// The command ring was momentarily full; retrying next frame converges.
    Transient,
    /// Anything else (bus exhaustion, install or build failure) - retrying
    /// without an edit would fail identically, so the head parks as usual.
    Permanent,
}

/// Hash of a part's bus wiring - every read/write's key, width and param.
/// Combined with the def's structural sig for the keep/replace decision: a
/// re-route that keeps the def (e.g. an instance inlet fed from a different
/// source) still respawns, crossfading onto the new buses.
fn wiring_hash(part: &ResolvedPart) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    for b in &part.bus_reads {
        (0u8, &b.key, b.channels, b.param).hash(&mut h);
    }
    for b in &part.bus_writes {
        (1u8, &b.key, b.channels, b.param).hash(&mut h);
    }
    h.finish()
}

/// Cue scope streams and allocate buses (so their indices are known), then
/// install and spawn one part's def - `Before` the given anchor when present,
/// else at the root group's tail. The synth spawns silent behind its fade
/// gains' baked `0.0` defaults; bus indices, scope bufnums and unbound
/// fade-to-unity are then set via `set_control` in one command-ring drain
/// (landing before the first audible block). Bound params re-send from node
/// state via the same-frame param sync. On failure, cleans up after itself
/// and reports whether retrying next frame can converge ([`SpawnError`]).
fn spawn_part(
    controller: &mut Controller,
    state: &mut HeadSynths,
    assets: &BufferBlobs,
    entity: Entity,
    part: ResolvedPart,
    wiring: u64,
    sample_rate: f64,
    anchor: Option<i32>,
) -> Result<PartSynth, SpawnError> {
    let now = Instant::now();
    let ResolvedPart {
        key,
        sig,
        def,
        params,
        monitors,
        gains,
        buffers,
        bus_writes,
        bus_reads,
        shapes: _,
    } = part;

    // Allocate buses and cue scope streams up front so their indices are known
    // for the post-spawn `set_control` drain below (no def mutation: the bus
    // indices and bufnums are no-lag control params, set live per block).
    // Whichever side of a bus spawns first allocates; the counterpart (spawned
    // later in topo order, or kept from a previous sync) looks the run up by
    // the same key.
    let mut set_after_spawn: Vec<(usize, f32)> = Vec::new();
    for binding in bus_writes.iter().chain(&bus_reads) {
        let bus_key = (entity, binding.key.clone());
        let Some(run) = state.bus_alloc.get_or_alloc(bus_key, binding.channels, now) else {
            log::error!("bevy_gantz_plyphon: private audio buses exhausted; part not spawned");
            return Err(SpawnError::Permanent);
        };
        set_after_spawn.push((binding.param, run.start as f32));
    }

    let mut scopes = Vec::new();
    for m in &monitors {
        let index = state.scope_alloc.alloc();
        let channels = m.channels.max(1);
        match controller.cue_scope(index, channels, sample_rate, CHUNK_FRAMES, NUM_CHUNKS) {
            Ok(consumer) => {
                set_after_spawn.push((m.bufnum_param, index as f32));
                scopes.push(ScopeSlot {
                    node_path: m.node_path.clone(),
                    size: m.size,
                    channels,
                    index,
                    consumer,
                })
            }
            Err(e) => {
                log::error!("bevy_gantz_plyphon: cue_scope failed: {e:?}");
                state.scope_alloc.free(index);
            }
        }
    }

    // Make each referenced asset resident (shared + refcounted) and wire the
    // node's driver-owned bufnum + rate. A missing/undecodable asset is wired to
    // a missing buffer (`-1`) so the node plays silence rather than a wrong buffer.
    let mut part_assets: Vec<ca::ContentAddr> = Vec::new();
    for binding in &buffers {
        match resolve_resident(controller, state, assets, &mut part_assets, binding.asset) {
            Some(bufnum) => {
                set_after_spawn.push((binding.bufnum_param, bufnum as f32));
                let rate = if sample_rate > 0.0 {
                    (binding.sample_rate / sample_rate) as f32
                } else {
                    1.0
                };
                set_after_spawn.push((binding.rate_param, rate));
            }
            None => set_after_spawn.push((binding.bufnum_param, -1.0)),
        }
    }

    let def_name = def.name.clone();
    let mut backend = Embedded::new(controller);
    // Install the def unless an identical one (same name AND structural sig) is
    // already installed. A structural edit to a stable-key region changes the
    // def while keeping the name, so the sig differs and we re-install
    // (retiring the old compiled def; running synths keep their own `Arc`).
    let sig_matches = state
        .shared_defs
        .get(&def_name)
        .is_some_and(|&(_, s)| s == sig);
    if !sig_matches {
        if let Err(e) = backend.install_synthdef((*def).clone()) {
            log::error!("bevy_gantz_plyphon: synthdef install failed: {e:?}");
            drop(backend);
            free_scopes(controller, &mut state.scope_alloc, scopes);
            free_buffers(controller, state, &part_assets, now);
            return Err(SpawnError::Permanent);
        }
    }
    state
        .shared_defs
        .entry(def_name.clone())
        .and_modify(|(rc, s)| {
            *rc += 1;
            *s = sig;
        })
        .or_insert((1, sig));
    let (target, action) = match anchor {
        Some(node) => (node, AddAction::Before),
        None => (ROOT_GROUP_ID, AddAction::Tail),
    };
    match backend.spawn(&def_name, target, action) {
        Ok(node_id) => {
            // Wire the synth in one command-ring drain: bus indices, scope
            // bufnums, then the unbound fade gains ramped to unity. All land
            // before the synth's first audible block (it spawns silent behind
            // the fade's baked `0.0` default).
            for (param, value) in &set_after_spawn {
                if let Err(e) = backend.set_control(node_id, *param, *value) {
                    log::error!("bevy_gantz_plyphon: post-spawn set_control failed: {e:?}");
                }
            }
            for g in &gains {
                if !params.iter().any(|b| b.index == g.index) {
                    if let Err(e) = backend.set_control(node_id, g.index, 1.0) {
                        log::error!("bevy_gantz_plyphon: fade-gain restore failed: {e:?}");
                    }
                }
            }
            let params = params
                .iter()
                .map(|b| ParamSlot {
                    node_path: b.node_path.clone(),
                    index: b.index,
                    last: None,
                })
                .collect();
            Ok(PartSynth {
                key,
                def_name,
                node_id,
                sig,
                wiring,
                params,
                scopes,
                gains,
                buffers: part_assets,
            })
        }
        Err(e) => {
            log::error!("bevy_gantz_plyphon: synth spawn failed: {e:?}");
            let spawn_err = match e {
                gantz_plyphon::BackendError::QueueFull => SpawnError::Transient,
                _ => SpawnError::Permanent,
            };
            drop(backend);
            release_def(controller, &mut state.shared_defs, &def_name);
            free_scopes(controller, &mut state.scope_alloc, scopes);
            free_buffers(controller, state, &part_assets, now);
            Err(spawn_err)
        }
    }
}

/// Begin fading `synth` out: ramp every gain to zero (each through its own lag)
/// and queue the synth - with its cued scope streams - for a deferred free once
/// the slowest ramp has died away. A synth with no gains (e.g. monitor-only) has
/// no audible output to de-click and is freed immediately.
fn fade_out(controller: &mut Controller, state: &mut HeadSynths, entity: Entity, synth: PartSynth) {
    if synth.gains.is_empty() {
        let _ = Embedded::new(controller).free_node(synth.node_id);
        free_scopes(controller, &mut state.scope_alloc, synth.scopes);
        // No fade to ride through: release the buffer refcounts now.
        free_buffers(controller, state, &synth.buffers, Instant::now());
        return;
    }
    let mut backend = Embedded::new(controller);
    let mut max_lag = 0.0f32;
    for g in &synth.gains {
        max_lag = max_lag.max(g.lag);
        if let Err(e) = backend.set_control(synth.node_id, g.index, 0.0) {
            log::error!("bevy_gantz_plyphon: fade-out set_control failed: {e:?}");
        }
    }
    // The buffers ride into the fading entry: their refcounts drop only at the
    // deadline (below), keeping the buffer resident through the crossfade so a
    // respawn on the same asset never reloads.
    state.fading.push(FadingSynth {
        entity,
        node_id: synth.node_id,
        deadline: Instant::now() + FADE_GRACE.max(Duration::from_secs_f32(max_lag)),
        scopes: synth.scopes,
        buffers: synth.buffers,
    });
}

/// Close each scope stream's cued recording slot and return its index to the
/// allocator for reuse (the `StreamConsumer`s drop with the vec).
fn free_scopes(controller: &mut Controller, scope_alloc: &mut ScopeAlloc, scopes: Vec<ScopeSlot>) {
    for scope in scopes {
        let _ = controller.close_recording(scope.index);
        scope_alloc.free(scope.index);
    }
}

/// Ensure `asset` is resident (installing its buffer from the content-addressed
/// blob on first use), take a refcount for the spawning part, and return its
/// bufnum - or `None` if the blob is missing/undecodable or installation failed
/// (the caller then wires the node to a missing buffer so it plays silence).
///
/// `part_assets` records the distinct assets this part already holds a refcount
/// for, so a part with two nodes playing the same asset shares one bufnum and
/// takes one refcount (released once when the synth is freed).
fn resolve_resident(
    controller: &mut Controller,
    state: &mut HeadSynths,
    assets: &BufferBlobs,
    part_assets: &mut Vec<ca::ContentAddr>,
    asset: ca::ContentAddr,
) -> Option<usize> {
    if let Some(rb) = state.resident.get(&asset) {
        let bufnum = rb.bufnum;
        if !part_assets.contains(&asset) {
            state.resident.get_mut(&asset).unwrap().refcount += 1;
            part_assets.push(asset);
        }
        return Some(bufnum);
    }
    let Some(blob) = assets.get(&asset) else {
        log::error!("bevy_gantz_plyphon: asset {asset} not in the registry; playing silence");
        return None;
    };
    let audio = gantz_plyphon::AudioAsset::decode(blob.as_ref())
        .map_err(|e| log::error!("bevy_gantz_plyphon: asset {asset} decode failed: {e}"))
        .ok()?;
    let bufnum = state.buffer_alloc.alloc();
    if let Err(e) = controller.buffer_set(bufnum, Box::new(audio.into())) {
        log::error!("bevy_gantz_plyphon: buffer_set failed for {asset}: {e:?}");
        state.buffer_alloc.free(bufnum, Instant::now());
        return None;
    }
    state.resident.insert(
        asset,
        ResidentBuffer {
            bufnum,
            refcount: 1,
        },
    );
    part_assets.push(asset);
    Some(bufnum)
}

/// Release each asset's refcount as a retired synth is freed; a buffer whose
/// last reference is gone is freed from the engine and its bufnum quarantined
/// (see [`BufferAlloc::free`]) before reuse.
fn free_buffers(
    controller: &mut Controller,
    state: &mut HeadSynths,
    assets: &[ca::ContentAddr],
    now: Instant,
) {
    for asset in assets {
        if let Some(rb) = state.resident.get_mut(asset) {
            rb.refcount -= 1;
            if rb.refcount == 0 {
                let bufnum = rb.bufnum;
                state.resident.remove(asset);
                let _ = controller.buffer_free(bufnum);
                state.buffer_alloc.free(bufnum, now);
            }
        }
    }
}

/// Decrement the refcount of `def_name`, freeing the def (retiring it on the
/// audio thread) only when the last reference drops. A shared variant def is
/// thus installed once and freed once its last instance is retired.
fn release_def(
    controller: &mut Controller,
    shared_defs: &mut HashMap<String, (usize, u64)>,
    def_name: &str,
) {
    let entry = shared_defs.entry(def_name.to_string()).or_insert((0, 0));
    entry.0 = entry.0.saturating_sub(1);
    if entry.0 == 0 {
        shared_defs.remove(def_name);
        let _ = Embedded::new(controller).free_synthdef(def_name);
    }
}

/// Whether this call is running on cpal's AudioWorklet audio thread.
///
/// cpal's AudioWorklet backend re-instantiates the wasm module on the audio
/// thread, re-running `main` there - an application `main` must return early
/// when this is `true`, or the whole app would boot again on the audio thread.
/// Always `false` off the web / without the `audioworklet` feature.
pub fn on_worklet_thread() -> bool {
    #[cfg(all(target_arch = "wasm32", feature = "audioworklet"))]
    {
        web_sys::window().is_none()
    }
    #[cfg(not(all(target_arch = "wasm32", feature = "audioworklet")))]
    {
        false
    }
}

/// The cpal host the output stream is built from: the platform default, or -
/// on the web with the `audioworklet` feature - cpal's AudioWorklet host (the
/// default there is the deprecated ScriptProcessor-based Web Audio host, which
/// runs audio on the main thread).
fn output_host() -> cpal::Host {
    #[cfg(all(target_arch = "wasm32", feature = "audioworklet"))]
    {
        cpal::host_from_id(cpal::HostId::AudioWorklet)
            .expect("AudioWorklet host unavailable (the page must be cross-origin isolated)")
    }
    #[cfg(not(all(target_arch = "wasm32", feature = "audioworklet")))]
    {
        cpal::default_host()
    }
}

/// Build the plyphon engine + cpal output stream from the default output device.
/// Returns `None` (and the app runs silently) if no device is available. `epoch`
/// is the shared monotonic clock the callback anchors the engine clock to;
/// `unit_registrars` register any custom units into the controller's registry
/// before the stream starts.
fn build_dsp_engine(epoch: EvalEpoch, unit_registrars: &[UnitRegistrar]) -> Option<DspEngine> {
    let host = output_host();
    let device = host.default_output_device()?;
    // cpal's `Device` `Display` is its name (there is no `name()` in 0.18).
    let device_name = device.to_string();
    let supported = device.default_output_config().ok()?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate as f64;

    let options = Options {
        sample_rate,
        output_channels: channels,
        ..Options::default()
    };
    // Private bus channels sit after the hardware output + input banks.
    let first_private_channel = options.output_channels + options.input_channels;
    let private_channels = options.audio_bus_channels;
    let (mut controller, nrt, world) = engine(options);

    // Register any custom units before the engine compiles/spawns a synth naming
    // them (the compiled GraphDef carries their fn-pointers to the audio thread).
    for register in unit_registrars {
        register(controller.registry_mut());
    }

    let stream = build_stream(&device, config, channels, sample_format, world, epoch)?;
    stream.play().ok()?;

    Some(DspEngine {
        controller,
        nrt,
        out_channels: channels,
        first_private_channel,
        private_channels,
        device: device_name,
        sample_rate,
        stream,
    })
}

/// Build the output stream, dispatching on the device's native sample format.
/// `world` is moved onto the audio thread and pulled once per callback.
fn build_stream(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    format: cpal::SampleFormat,
    world: World,
    epoch: EvalEpoch,
) -> Option<cpal::Stream> {
    match format {
        cpal::SampleFormat::F32 => build_typed::<f32>(device, config, channels, world, epoch),
        cpal::SampleFormat::I16 => build_typed::<i16>(device, config, channels, world, epoch),
        cpal::SampleFormat::U16 => build_typed::<u16>(device, config, channels, world, epoch),
        other => {
            log::error!("bevy_gantz_plyphon: unsupported sample format {other:?}");
            None
        }
    }
}

/// Construct a typed output stream, reblocking the engine's `f32` fill into the
/// device's sample format `T`.
fn build_typed<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    mut world: World,
    epoch: EvalEpoch,
) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    // Reused interleaved `f32` scratch; the engine writes it, then we convert.
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            config,
            move |output: &mut [T], info: &cpal::OutputCallbackInfo| {
                scratch.clear();
                scratch.resize(output.len(), 0.0);
                // Natively, anchor the engine clock to this buffer's heard-time on the
                // shared monotonic epoch (`now` + the callback-to-playback latency), so
                // scheduled control updates resolve to the right sample even as the audio
                // device clock drifts against the epoch. On the web there is no clock on
                // cpal's AudioWorklet thread (no `performance`/`window` in an
                // `AudioWorkletGlobalScope`), so the engine clock free-runs at the nominal
                // rate instead - control times are then relative to engine start, matching
                // plyphon's web audio path.
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let ts = info.timestamp();
                    let ahead = ts.playback.duration_since(ts.callback).as_secs_f64();
                    let buffer_time = osc(epoch.now_secs() + ahead);
                    world.fill_at(&mut scratch, channels, buffer_time);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (info, &epoch);
                    world.fill(&mut scratch, channels);
                }
                for (o, s) in output.iter_mut().zip(scratch.iter()) {
                    *o = T::from_sample(*s);
                }
            },
            |err| log::error!("bevy_gantz_plyphon: audio stream error: {err}"),
            None,
        )
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fading entry with no scope streams, due at `deadline`.
    fn fade(entity: Entity, deadline: Instant) -> FadingSynth {
        FadingSynth {
            entity,
            node_id: 0,
            deadline,
            scopes: Vec::new(),
            buffers: Vec::new(),
        }
    }

    /// `secs` seconds after `base`.
    fn later(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    fn entities(n: usize) -> Vec<Entity> {
        let mut world = bevy_ecs::world::World::new();
        (0..n).map(|_| world.spawn_empty().id()).collect()
    }

    /// A one-asset store plus its address, for the resident-buffer tests.
    fn one_asset() -> (BufferBlobs, ca::ContentAddr) {
        let audio = gantz_plyphon::AudioAsset::from_interleaved(vec![0.5; 8], 1, 48_000.0);
        let addr = audio.addr();
        let bytes = ca::Bytes::from(audio.encode());
        (std::iter::once((addr, bytes)).collect(), addr)
    }

    fn test_controller() -> Controller {
        plyphon::engine(plyphon::Options::default()).0
    }

    /// A freed bufnum is quarantined (not reused immediately) and only returns
    /// to the pool once its grace has passed.
    #[test]
    fn buffer_alloc_quarantines_then_reuses() {
        let mut a = BufferAlloc::default();
        let b0 = a.alloc();
        let _b1 = a.alloc();
        let base = Instant::now();
        a.free(b0, base);
        // Immediately, a fresh index is handed out - not the quarantined one.
        assert_ne!(a.alloc(), b0);
        // A sweep before the grace expires keeps it quarantined.
        a.sweep(base);
        assert_ne!(a.alloc(), b0);
        // After the grace, the freed index returns to the pool.
        a.sweep(base + BUFFER_GRACE + Duration::from_millis(1));
        assert_eq!(a.alloc(), b0);
    }

    /// An asset shared by two synths is installed once, refcounted, and freed
    /// only when the last reference drops.
    #[test]
    fn resident_buffer_shares_and_refcounts() {
        let mut controller = test_controller();
        let mut state = HeadSynths::default();
        let (assets, addr) = one_asset();

        let mut a_refs = Vec::new();
        let bufnum =
            resolve_resident(&mut controller, &mut state, &assets, &mut a_refs, addr).unwrap();
        assert_eq!(a_refs, vec![addr]);
        assert_eq!(state.resident.get(&addr).unwrap().refcount, 1);

        // A second synth shares the same bufnum and bumps the refcount.
        let mut b_refs = Vec::new();
        let bufnum2 =
            resolve_resident(&mut controller, &mut state, &assets, &mut b_refs, addr).unwrap();
        assert_eq!(bufnum2, bufnum);
        assert_eq!(state.resident.get(&addr).unwrap().refcount, 2);

        // Freeing the first keeps the buffer resident for the second.
        free_buffers(&mut controller, &mut state, &a_refs, Instant::now());
        assert_eq!(state.resident.get(&addr).unwrap().refcount, 1);

        // Freeing the last drops it and quarantines the bufnum.
        free_buffers(&mut controller, &mut state, &b_refs, Instant::now());
        assert!(!state.resident.contains_key(&addr));
    }

    /// Two references to the same asset within one part take a single refcount.
    #[test]
    fn resolve_resident_dedups_within_a_part() {
        let mut controller = test_controller();
        let mut state = HeadSynths::default();
        let (assets, addr) = one_asset();
        let mut refs = Vec::new();
        resolve_resident(&mut controller, &mut state, &assets, &mut refs, addr).unwrap();
        resolve_resident(&mut controller, &mut state, &assets, &mut refs, addr).unwrap();
        assert_eq!(refs, vec![addr]);
        assert_eq!(state.resident.get(&addr).unwrap().refcount, 1);
    }

    /// An asset absent from the store resolves to `None` (the node plays silence).
    #[test]
    fn resolve_resident_missing_asset_is_none() {
        let mut controller = test_controller();
        let mut state = HeadSynths::default();
        let (assets, _addr) = one_asset();
        let mut refs = Vec::new();
        let missing = ca::blob_addr(b"absent");
        assert!(
            resolve_resident(&mut controller, &mut state, &assets, &mut refs, missing).is_none()
        );
        assert!(refs.is_empty());
    }

    /// Entries past their deadline drain; the rest stay, in order.
    #[test]
    fn expire_fades_drains_past_deadline() {
        let e = entities(1);
        let base = Instant::now();
        let mut fading = vec![fade(e[0], base), fade(e[0], later(base, 1))];
        let expired = expire_fades(&mut fading, later(base, 0));
        assert_eq!(expired.len(), 1);
        assert_eq!(fading.len(), 1);
        assert!(fading[0].deadline > base);
    }

    /// A head's backlog over the cap retires its OLDEST entries early, leaving
    /// at most `MAX_FADING_PER_HEAD`; other heads' entries are untouched.
    #[test]
    fn expire_fades_caps_per_head_backlog() {
        let e = entities(2);
        let base = Instant::now();
        let mut fading: Vec<FadingSynth> = (0..4).map(|i| fade(e[0], later(base, 1 + i))).collect();
        fading.push(fade(e[1], later(base, 1)));
        let expired = expire_fades(&mut fading, base);
        // The two oldest of e0's four go; e1's single entry stays.
        assert_eq!(expired.len(), 4 - MAX_FADING_PER_HEAD);
        assert!(expired.iter().all(|f| f.entity == e[0]));
        assert_eq!(
            fading.iter().filter(|f| f.entity == e[0]).count(),
            MAX_FADING_PER_HEAD,
        );
        assert_eq!(fading.iter().filter(|f| f.entity == e[1]).count(), 1);
        // The kept entries are the newest (latest deadlines).
        assert!(
            fading
                .iter()
                .filter(|f| f.entity == e[0])
                .all(|f| f.deadline >= later(base, 3)),
        );
    }

    /// Under the cap and before any deadline, nothing drains.
    #[test]
    fn expire_fades_keeps_recent_under_cap() {
        let e = entities(1);
        let base = Instant::now();
        let mut fading = vec![fade(e[0], later(base, 1)), fade(e[0], later(base, 2))];
        assert!(expire_fades(&mut fading, base).is_empty());
        assert_eq!(fading.len(), 2);
    }

    /// Consecutive-run allocation, key stability, and width-change realloc.
    #[test]
    fn bus_alloc_allocates_stable_consecutive_runs() {
        let e = entities(1);
        let base = Instant::now();
        let mut alloc = BusAlloc::new(4, 12);
        let a = alloc
            .get_or_alloc((e[0], BusKey::Bus(vec![1])), 2, base)
            .expect("2 channels");
        assert_eq!(a, Run { start: 4, len: 2 });
        let b = alloc
            .get_or_alloc((e[0], BusKey::Bus(vec![2])), 3, base)
            .expect("3 channels");
        assert_eq!(b, Run { start: 6, len: 3 });
        // Same key + width: the same run (an unchanged region's baked channel
        // stays valid).
        assert_eq!(
            alloc.get_or_alloc((e[0], BusKey::Bus(vec![1])), 2, base),
            Some(a)
        );
        // A width change re-allocates; the old run is quarantined, not reused.
        let a2 = alloc
            .get_or_alloc((e[0], BusKey::Bus(vec![1])), 4, base)
            .expect("4 channels");
        assert_eq!(a2, Run { start: 9, len: 4 });
        assert!(alloc.graveyard.iter().any(|(run, _)| *run == a));
    }

    /// Quarantined runs return to the free list only after their deadline, and
    /// coalesce so wide runs stay allocatable.
    #[test]
    fn bus_alloc_quarantines_then_coalesces() {
        let e = entities(1);
        let base = Instant::now();
        let mut alloc = BusAlloc::new(0, 4);
        let a = alloc
            .get_or_alloc((e[0], BusKey::Bus(vec![1])), 2, base)
            .expect("a");
        let b = alloc
            .get_or_alloc((e[0], BusKey::Bus(vec![2])), 2, base)
            .expect("b");
        assert_eq!((a.start, b.start), (0, 2));
        // Exhausted while both are live.
        assert_eq!(
            alloc.get_or_alloc((e[0], BusKey::Bus(vec![3])), 1, base),
            None
        );
        // Released runs stay quarantined until the deadline passes...
        alloc.release_head(e[0], base);
        alloc.sweep(base);
        assert_eq!(
            alloc.get_or_alloc((e[0], BusKey::Bus(vec![3])), 1, base),
            None
        );
        // ...then coalesce back into one allocatable 4-wide run.
        alloc.sweep(base + FADE_GRACE * 2);
        assert_eq!(
            alloc.get_or_alloc((e[0], BusKey::Bus(vec![3])), 4, base),
            Some(Run { start: 0, len: 4 }),
        );
    }

    /// A `~sinosc -> ~out` graph spawns through `spawn_part` and sounds: the
    /// fade gain (baked at `0.0`) is ramped to unity via `set_control`, and the
    /// tone is audible after the fade lag. Guards the driver's runtime spawn
    /// path (the GUI's path) end to end with an offline engine.
    #[test]
    fn spawn_part_sounds_sin_out() {
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        use gantz_plyphon::flatten::{Flat, RefKind, flatten};

        let mut g = Graph::<TestN>::default();
        let s = g.add_node(TestN::SinOsc(gantz_plyphon::SinOsc::default()));
        let o = g.add_node(TestN::Out(gantz_plyphon::Out::default()));
        g.add_edge(s, o, Edge::new(0.into(), 0.into()));
        let resolve =
            |_: &TestN| -> Option<(gantz_ca::ContentAddr, RefKind, Option<&Graph<TestN>>)> { None };
        let flat: Graph<Flat<&TestN>> = flatten(&|_| None, &g, &resolve).expect("flatten");

        let mut cache = DefCache::new();
        let template = derive_template(&flat, 1, &|_| None, &mut cache).expect("derive");
        let part = instantiate(&template, &cache).into_iter().next().unwrap();
        let wiring = wiring_hash(&part);

        let (mut controller, _nrt, mut world) = plyphon::engine(plyphon::Options {
            sample_rate: 48_000.0,
            output_channels: 1,
            ..plyphon::Options::default()
        });
        let mut state = HeadSynths::default();
        let assets = BufferBlobs::default();
        let entity = entities(1)[0];
        let synth = spawn_part(
            &mut controller,
            &mut state,
            &assets,
            entity,
            part,
            wiring,
            48_000.0,
            None,
        )
        .expect("spawn_part");
        assert_eq!(synth.gains.len(), 1, "the out carries a fade gain");

        let mut out = vec![0.0f32; 48_000 / 2];
        for block in out.chunks_mut(64) {
            world.fill(block, 1);
        }
        let rms = (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt();
        assert!(
            rms > 0.05,
            "sin -> out must sound via spawn_part: rms={rms}"
        );
    }

    /// End-to-end: a `~playbuf -> ~out` graph plays a content-addressed asset.
    /// Exercises the whole chain - node -> derive -> `BufferBinding` -> resident
    /// install -> `PlayBuf` reads the buffer -> `Out` - and confirms it sounds.
    #[test]
    fn playbuf_sounds_through_out() {
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        use gantz_plyphon::flatten::{Flat, RefKind, flatten};

        // An alternating +/-0.5 waveform (nonzero RMS), content-addressed and
        // placed in the store the driver reads.
        let samples = (0..64)
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        let audio = gantz_plyphon::AudioAsset::from_interleaved(samples, 1, 48_000.0);
        let addr = audio.addr();
        let bytes = ca::Bytes::from(audio.encode());
        let assets: BufferBlobs = std::iter::once((addr, bytes)).collect();

        let mut g = Graph::<TestN>::default();
        let p = g.add_node(TestN::PlayBuf(gantz_plyphon::PlayBuf::new(
            addr, 1, 48_000.0,
        )));
        let o = g.add_node(TestN::Out(gantz_plyphon::Out::default()));
        g.add_edge(p, o, Edge::new(0.into(), 0.into()));
        let resolve =
            |_: &TestN| -> Option<(gantz_ca::ContentAddr, RefKind, Option<&Graph<TestN>>)> { None };
        let flat: Graph<Flat<&TestN>> = flatten(&|_| None, &g, &resolve).expect("flatten");

        let mut cache = DefCache::new();
        let template = derive_template(&flat, 1, &|_| None, &mut cache).expect("derive");
        let part = instantiate(&template, &cache).into_iter().next().unwrap();
        let wiring = wiring_hash(&part);

        let (mut controller, _nrt, mut world) = plyphon::engine(plyphon::Options {
            sample_rate: 48_000.0,
            output_channels: 1,
            ..plyphon::Options::default()
        });
        let mut state = HeadSynths::default();
        let entity = entities(1)[0];
        let synth = spawn_part(
            &mut controller,
            &mut state,
            &assets,
            entity,
            part,
            wiring,
            48_000.0,
            None,
        )
        .expect("spawn_part");
        // The asset was made resident with a single refcount, held by this synth.
        assert_eq!(state.resident.get(&addr).map(|r| r.refcount), Some(1));
        assert_eq!(synth.buffers, vec![addr]);

        let mut out = vec![0.0f32; 48_000 / 2];
        for block in out.chunks_mut(64) {
            world.fill(block, 1);
        }
        let rms = (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt();
        assert!(rms > 0.05, "playbuf -> out must sound: rms={rms}");
    }

    /// Multi-frame `structural_sync`: a `~sinosc -> ~out` head graph sounds,
    /// and adding unconnected `inlet`/`outlet` nodes (which dissolve in the
    /// flattener) keeps the tone audible (the region is kept or respawned with
    /// the fade ramped in). Regression guard for the GUI's reported "no sound
    /// with inlet/outlet" issue. Mimics the GUI exactly: flatten, then sync.
    #[test]
    fn structural_sync_keeps_sound_with_inlets() {
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        use gantz_plyphon::flatten::{Flat, RefKind, flatten};

        fn flatten_no_refs(g: &Graph<TestN>) -> Graph<Flat<&TestN>> {
            let resolve =
                |_: &TestN| -> Option<(gantz_ca::ContentAddr, RefKind, Option<&Graph<TestN>>)> {
                    None
                };
            flatten(&|_| None, g, &resolve).expect("flatten")
        }

        let (mut controller, _nrt, mut world) = plyphon::engine(plyphon::Options {
            sample_rate: 48_000.0,
            output_channels: 1,
            ..plyphon::Options::default()
        });
        let mut state = HeadSynths::default();
        let entity = entities(1)[0];

        // Frame 1: `~sinosc -> ~out`.
        let mut g1 = Graph::<TestN>::default();
        let s = g1.add_node(TestN::SinOsc(gantz_plyphon::SinOsc::default()));
        let o = g1.add_node(TestN::Out(gantz_plyphon::Out::default()));
        g1.add_edge(s, o, Edge::new(0.into(), 0.into()));
        let ca1 = graph_addr(&g1);
        let flat1 = flatten_no_refs(&g1);
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            ca1,
            &flat1,
            &HashMap::new(),
            1,
            48_000.0,
        );
        let mut out = vec![0.0f32; 48_000 / 4];
        for block in out.chunks_mut(64) {
            world.fill(block, 1);
        }
        let rms1 = (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt();
        assert!(rms1 > 0.05, "frame 1 must sound: rms={rms1}");

        // Frame 2: add unconnected `inlet`/`outlet` (different graph address).
        let mut g2 = Graph::<TestN>::default();
        let s = g2.add_node(TestN::SinOsc(gantz_plyphon::SinOsc::default()));
        let o = g2.add_node(TestN::Out(gantz_plyphon::Out::default()));
        let _i = g2.add_node(TestN::Inlet);
        let _o2 = g2.add_node(TestN::Outlet);
        g2.add_edge(s, o, Edge::new(0.into(), 0.into()));
        let ca2 = graph_addr(&g2);
        assert_ne!(ca1, ca2, "adding nodes changes the graph address");
        let flat2 = flatten_no_refs(&g2);
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            ca2,
            &flat2,
            &HashMap::new(),
            1,
            48_000.0,
        );
        // Render past the crossfade (the respawn fades in over FADE_LAG).
        let mut out = vec![0.0f32; 48_000 / 2];
        for block in out.chunks_mut(64) {
            world.fill(block, 1);
        }
        let rms2 = (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt();
        assert!(rms2 > 0.05, "frame 2 (with inlets) must sound: rms={rms2}");
    }

    /// A structural edit within a stable-key region (connecting `~sinosc` into
    /// an existing `~pack -> ~unpack -> ~out` chain) re-installs the def and
    /// the synth plays the NEW (sounding) def. Regression guard for the GUI's
    /// reported "~pack/~unpack no longer work" issue: the Phase-4 refcounting
    /// must not skip `install_synthdef` when the def changed (same name, new sig).
    #[test]
    fn structural_edit_in_stable_region_reinstalls_def() {
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        use gantz_plyphon::flatten::{Flat, RefKind, flatten};

        fn flatten_no_refs(g: &Graph<TestN>) -> Graph<Flat<&TestN>> {
            let resolve =
                |_: &TestN| -> Option<(gantz_ca::ContentAddr, RefKind, Option<&Graph<TestN>>)> {
                    None
                };
            flatten(&|_| None, g, &resolve).expect("flatten")
        }

        let (mut controller, _nrt, mut world) = plyphon::engine(plyphon::Options {
            sample_rate: 48_000.0,
            output_channels: 1,
            ..plyphon::Options::default()
        });
        let mut state = HeadSynths::default();
        let entity = entities(1)[0];

        // Frame 1: `~pack -> ~unpack -> ~out` (pack input unconnected -> silence).
        let mut g1 = Graph::<TestN>::default();
        let pk = g1.add_node(TestN::Pack(gantz_plyphon::Pack::default()));
        let up = g1.add_node(TestN::Unpack(gantz_plyphon::Unpack::default()));
        let o = g1.add_node(TestN::Out(gantz_plyphon::Out::default()));
        g1.add_edge(pk, up, Edge::new(0.into(), 0.into()));
        g1.add_edge(up, o, Edge::new(0.into(), 0.into()));
        let ca1 = graph_addr(&g1);
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            ca1,
            &flatten_no_refs(&g1),
            &HashMap::new(),
            1,
            48_000.0,
        );
        let mut out = vec![0.0f32; 48_000 / 8];
        for block in out.chunks_mut(64) {
            world.fill(block, 1);
        }
        let rms1 = (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt();
        assert!(
            rms1 < 1e-3,
            "frame 1 (unconnected pack) must be silent: rms={rms1}"
        );

        // Frame 2: connect `~sinosc -> ~pack` (same `~out` path -> stable key,
        // same name, but the def CHANGED: now carries a SinOsc). Must re-install.
        let mut g2 = g1.clone();
        let s = g2.add_node(TestN::SinOsc(gantz_plyphon::SinOsc::default()));
        g2.add_edge(s, pk, Edge::new(0.into(), 0.into()));
        let ca2 = graph_addr(&g2);
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            ca2,
            &flatten_no_refs(&g2),
            &HashMap::new(),
            1,
            48_000.0,
        );
        let mut out = vec![0.0f32; 48_000 / 2];
        for block in out.chunks_mut(64) {
            world.fill(block, 1);
        }
        let rms2 = (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt();
        assert!(
            rms2 > 0.05,
            "frame 2 (sinosc connected) must sound: rms={rms2}"
        );
    }

    /// A minimal node enum for driver tests: DSP sinks/sources plus the nesting
    /// markers (`Inlet`/`Outlet`) and a graph ref carrying its child's content
    /// address, arity and `inline` flag.
    #[derive(Clone)]
    enum TestN {
        SinOsc(gantz_plyphon::SinOsc),
        Out(gantz_plyphon::Out),
        Pack(gantz_plyphon::Pack),
        Unpack(gantz_plyphon::Unpack),
        PlayBuf(gantz_plyphon::PlayBuf),
        Inlet,
        Outlet,
        Ref(gantz_ca::ContentAddr, usize, usize, bool),
    }

    /// The graph's erased (data-layer) address: the same scheme the registry
    /// uses for graph identity, so structural edits change the address and
    /// identical graphs share one.
    fn graph_addr(g: &gantz_core::node::graph::Graph<TestN>) -> gantz_ca::GraphAddr {
        use gantz_ca::{Datum, NodeData};
        use gantz_core::data::erase_node_typed;
        let dg: gantz_ca::DataGraph = g.map(
            |_, n| match n {
                TestN::SinOsc(s) => erase_node_typed(s).unwrap(),
                TestN::Out(o) => erase_node_typed(o).unwrap(),
                TestN::Pack(p) => erase_node_typed(p).unwrap(),
                TestN::Unpack(u) => erase_node_typed(u).unwrap(),
                TestN::PlayBuf(p) => erase_node_typed(p).unwrap(),
                TestN::Inlet => {
                    erase_node_typed(&gantz_core::node::graph::Inlet::default()).unwrap()
                }
                TestN::Outlet => {
                    erase_node_typed(&gantz_core::node::graph::Outlet::default()).unwrap()
                }
                TestN::Ref(ca, n_in, n_out, inline) => {
                    let mut nd = NodeData::new(
                        "test.ref",
                        Datum::Map(vec![
                            ("addr".to_string(), Datum::Str(ca.to_string())),
                            ("inline".to_string(), Datum::Bool(*inline)),
                            ("n_in".to_string(), Datum::U64(*n_in as u64)),
                            ("n_out".to_string(), Datum::U64(*n_out as u64)),
                        ]),
                    );
                    nd.refs.push(*ca);
                    nd.canonicalize();
                    nd
                }
            },
            |_, e| *e,
        );
        gantz_ca::graph_addr(&dg)
    }

    impl gantz_plyphon::ToNodeDsp for TestN {
        fn to_node_dsp(&self) -> Option<&dyn gantz_plyphon::NodeDsp> {
            match self {
                TestN::SinOsc(s) => Some(s),
                TestN::Out(o) => Some(o),
                TestN::Pack(p) => Some(p),
                TestN::Unpack(u) => Some(u),
                TestN::PlayBuf(p) => Some(p),
                TestN::Inlet | TestN::Outlet | TestN::Ref(..) => None,
            }
        }
    }

    impl gantz_core::Node for TestN {
        fn expr(&self, _ctx: gantz_core::node::ExprCtx<'_, '_>) -> gantz_core::node::ExprResult {
            gantz_core::node::parse_expr("'()")
        }
        fn inlet(&self, _ctx: gantz_core::node::MetaCtx) -> bool {
            matches!(self, TestN::Inlet)
        }
        fn outlet(&self, _ctx: gantz_core::node::MetaCtx) -> bool {
            matches!(self, TestN::Outlet)
        }
        fn n_inputs(&self, _ctx: gantz_core::node::MetaCtx) -> usize {
            match self {
                TestN::Ref(_, n_in, _, _) => *n_in,
                _ => 0,
            }
        }
        fn n_outputs(&self, _ctx: gantz_core::node::MetaCtx) -> usize {
            match self {
                TestN::Ref(_, _, n_out, _) => *n_out,
                _ => 0,
            }
        }
    }

    /// Flatten a head graph over `map`'s children (refs lower per their
    /// `inline` flag) and pre-flatten every child for the derivation resolver.
    #[allow(clippy::type_complexity)]
    fn flatten_head<'g>(
        g: &'g gantz_core::node::graph::Graph<TestN>,
        map: &'g HashMap<gantz_ca::ContentAddr, gantz_core::node::graph::Graph<TestN>>,
    ) -> (
        gantz_core::node::graph::Graph<gantz_plyphon::Flat<&'g TestN>>,
        HashMap<
            gantz_ca::ContentAddr,
            gantz_core::node::graph::Graph<gantz_plyphon::Flat<&'g TestN>>,
        >,
    ) {
        use gantz_core::node::graph::Graph;
        use gantz_plyphon::flatten::{RefKind, flatten};
        fn resolver<'g>(
            map: &'g HashMap<gantz_ca::ContentAddr, Graph<TestN>>,
        ) -> impl Fn(&TestN) -> Option<(gantz_ca::ContentAddr, RefKind, Option<&'g Graph<TestN>>)> + 'g
        {
            move |n| match n {
                TestN::Ref(ca, _, _, inline) => {
                    let kind = if *inline {
                        RefKind::Inline
                    } else {
                        RefKind::Instance
                    };
                    Some((*ca, kind, map.get(ca)))
                }
                _ => None,
            }
        }
        let resolve = resolver(map);
        let flat = flatten(&|_| None, g, &resolve).expect("flatten head");
        let children = map
            .iter()
            .map(|(ca, child)| {
                let resolve = resolver(map);
                (
                    *ca,
                    flatten(&|_| None, child, &resolve).expect("flatten child"),
                )
            })
            .collect();
        (flat, children)
    }

    /// Render `frames` mono frames and return their RMS.
    fn render_rms(world: &mut plyphon::World, frames: usize) -> f32 {
        let mut out = vec![0.0f32; frames];
        for block in out.chunks_mut(64) {
            world.fill(block, 1);
        }
        (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt()
    }

    /// A self-contained child: `~sinosc -> ~out`.
    fn sine_out_child() -> gantz_core::node::graph::Graph<TestN> {
        use gantz_core::edge::Edge;
        let mut g = gantz_core::node::graph::Graph::<TestN>::default();
        let s = g.add_node(TestN::SinOsc(gantz_plyphon::SinOsc::default()));
        let o = g.add_node(TestN::Out(gantz_plyphon::Out::default()));
        g.add_edge(s, o, Edge::new(0.into(), 0.into()));
        g
    }

    fn test_engine() -> (Controller, Nrt, plyphon::World) {
        plyphon::engine(plyphon::Options {
            sample_rate: 48_000.0,
            output_channels: 1,
            ..plyphon::Options::default()
        })
    }

    /// Two instances of one self-contained child both sound (louder than one),
    /// sharing ONE installed def with refcount 2.
    #[test]
    fn two_instances_of_one_child_both_sound() {
        use gantz_core::node::graph::Graph;

        let ca = gantz_ca::ContentAddr([7u8; 32]);
        let map = HashMap::from([(ca, sine_out_child())]);

        // One instance, for the loudness baseline.
        let mut g1 = Graph::<TestN>::default();
        g1.add_node(TestN::Ref(ca, 0, 0, false));
        let (flat1, children) = flatten_head(&g1, &map);
        let (mut controller, _nrt, mut world) = test_engine();
        let mut state = HeadSynths::default();
        let entity = entities(1)[0];
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            graph_addr(&g1),
            &flat1,
            &children,
            1,
            48_000.0,
        );
        let rms_one = render_rms(&mut world, 48_000 / 2);
        assert!(rms_one > 0.05, "one instance must sound: rms={rms_one}");

        // Two instances in a fresh engine.
        let mut g2 = Graph::<TestN>::default();
        g2.add_node(TestN::Ref(ca, 0, 0, false));
        g2.add_node(TestN::Ref(ca, 0, 0, false));
        let (flat2, children) = flatten_head(&g2, &map);
        let (mut controller, _nrt, mut world) = test_engine();
        let mut state = HeadSynths::default();
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            graph_addr(&g2),
            &flat2,
            &children,
            1,
            48_000.0,
        );
        assert_eq!(state.heads[&entity].parts.len(), 2, "two instance spawns");
        assert_eq!(
            state.shared_defs.len(),
            1,
            "one shared def: {:?}",
            state.shared_defs.keys().collect::<Vec<_>>(),
        );
        let (rc, _) = state.shared_defs.values().next().unwrap();
        assert_eq!(*rc, 2, "both instances hold the def");
        let rms_two = render_rms(&mut world, 48_000 / 2);
        assert!(
            rms_two > rms_one * 1.5,
            "two in-phase instances sum louder: one={rms_one} two={rms_two}",
        );
    }

    /// Editing the child respawns exactly its instances: the head's own region
    /// keeps its synth (node id unchanged) while both instance spawns are
    /// replaced and the old pair fade out.
    #[test]
    fn child_edit_respawns_only_its_instances() {
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;

        let ca1 = gantz_ca::ContentAddr([1u8; 32]);
        // The edited child: structurally different (an extra `~pack` stage).
        let ca2 = gantz_ca::ContentAddr([2u8; 32]);
        let mut child2 = Graph::<TestN>::default();
        let s = child2.add_node(TestN::SinOsc(gantz_plyphon::SinOsc::default()));
        let pk = child2.add_node(TestN::Pack(gantz_plyphon::Pack::default()));
        let o = child2.add_node(TestN::Out(gantz_plyphon::Out::default()));
        child2.add_edge(s, pk, Edge::new(0.into(), 0.into()));
        child2.add_edge(pk, o, Edge::new(0.into(), 0.into()));
        let map = HashMap::from([(ca1, sine_out_child()), (ca2, child2)]);

        // Head: its own region (sin -> out) + two instances of the child.
        let head = |ca: gantz_ca::ContentAddr| {
            let mut g = Graph::<TestN>::default();
            let s = g.add_node(TestN::SinOsc(gantz_plyphon::SinOsc::default()));
            let o = g.add_node(TestN::Out(gantz_plyphon::Out::default()));
            g.add_edge(s, o, Edge::new(0.into(), 0.into()));
            g.add_node(TestN::Ref(ca, 0, 0, false));
            g.add_node(TestN::Ref(ca, 0, 0, false));
            g
        };

        let (mut controller, _nrt, _world) = test_engine();
        let mut state = HeadSynths::default();
        let entity = entities(1)[0];
        let g1 = head(ca1);
        let (flat1, children) = flatten_head(&g1, &map);
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            graph_addr(&g1),
            &flat1,
            &children,
            1,
            48_000.0,
        );
        let before: HashMap<u64, i32> = state.heads[&entity]
            .parts
            .iter()
            .map(|p| (p.key, p.node_id))
            .collect();
        assert_eq!(before.len(), 3, "own region + two instances");

        let g2 = head(ca2);
        let (flat2, children) = flatten_head(&g2, &map);
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            graph_addr(&g2),
            &flat2,
            &children,
            1,
            48_000.0,
        );
        let after: HashMap<u64, i32> = state.heads[&entity]
            .parts
            .iter()
            .map(|p| (p.key, p.node_id))
            .collect();
        assert_eq!(after.len(), 3);

        // Exactly one part survives by key - the head's own region - with its
        // synth untouched. The instance parts (child CA in their key) are new,
        // and the two retired spawns are fading.
        let kept: Vec<_> = after
            .iter()
            .filter(|(k, _)| before.contains_key(k))
            .collect();
        assert_eq!(kept.len(), 1, "only the head's own region matches by key");
        let (kept_key, kept_id) = kept[0];
        assert_eq!(
            before[kept_key], *kept_id,
            "the kept region never respawned"
        );
        assert_eq!(state.fading.len(), 2, "both old instance spawns fade out");
    }

    /// Toggling `inline` on the ref switches the lowering live: the instanced
    /// spawn crossfades to the spliced one, audibly on both sides.
    #[test]
    fn inline_toggle_switches_lowering_live() {
        use gantz_core::node::graph::Graph;

        let ca = gantz_ca::ContentAddr([3u8; 32]);
        let map = HashMap::from([(ca, sine_out_child())]);
        let head = |inline: bool| {
            let mut g = Graph::<TestN>::default();
            g.add_node(TestN::Ref(ca, 0, 0, inline));
            g
        };

        let (mut controller, _nrt, mut world) = test_engine();
        let mut state = HeadSynths::default();
        let entity = entities(1)[0];

        let g1 = head(false);
        let (flat1, children) = flatten_head(&g1, &map);
        assert!(
            flat1
                .node_indices()
                .any(|n| matches!(flat1[n], gantz_plyphon::Flat::Instance { .. })),
            "instanced by default",
        );
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            graph_addr(&g1),
            &flat1,
            &children,
            1,
            48_000.0,
        );
        let rms1 = render_rms(&mut world, 48_000 / 4);
        assert!(rms1 > 0.05, "instanced lowering sounds: rms={rms1}");

        let g2 = head(true);
        let (flat2, children) = flatten_head(&g2, &map);
        assert!(
            flat2
                .node_indices()
                .all(|n| !matches!(flat2[n], gantz_plyphon::Flat::Instance { .. })),
            "inline splices",
        );
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            graph_addr(&g2),
            &flat2,
            &children,
            1,
            48_000.0,
        );
        // Render across the crossfade: the tone stays audible throughout.
        let rms2 = render_rms(&mut world, 48_000 / 2);
        assert!(
            rms2 > 0.05,
            "inline lowering sounds after the switch: rms={rms2}"
        );
        assert_eq!(state.heads[&entity].parts.len(), 1);
    }

    /// A param edit lands on ONE instance's synth: setting one spawn's freq to
    /// zero leaves its sibling sounding (the shared def's param index is per
    /// synth), and the sibling's slot state is untouched.
    #[test]
    fn param_edit_on_one_instance_leaves_sibling() {
        use gantz_core::node::graph::Graph;

        let ca = gantz_ca::ContentAddr([5u8; 32]);
        let map = HashMap::from([(ca, sine_out_child())]);
        let mut g = Graph::<TestN>::default();
        g.add_node(TestN::Ref(ca, 0, 0, false));
        g.add_node(TestN::Ref(ca, 0, 0, false));
        let (flat, children) = flatten_head(&g, &map);

        let (mut controller, _nrt, mut world) = test_engine();
        let mut state = HeadSynths::default();
        let entity = entities(1)[0];
        structural_sync(
            &mut controller,
            &mut state,
            &Default::default(),
            entity,
            graph_addr(&g),
            &flat,
            &children,
            1,
            48_000.0,
        );
        let rms_both = render_rms(&mut world, 48_000 / 2);

        // Silence the FIRST instance's sine via its own synth's freq param
        // (the same set_control the driver's param sync issues). The slots
        // pair absolute node paths with def-local indices: both instances
        // share the index, but each has its own node id.
        let parts = &state.heads[&entity].parts;
        assert_eq!(parts.len(), 2);
        let (a, b) = (&parts[0], &parts[1]);
        assert_ne!(a.node_id, b.node_id);
        assert_ne!(a.params[0].node_path, b.params[0].node_path);
        assert_eq!(a.params[0].index, b.params[0].index, "shared def index");
        let freq = a
            .params
            .iter()
            .find(|s| s.node_path.len() == 2)
            .expect("the nested sine's param slot");
        Embedded::new(&mut controller)
            .set_control(a.node_id, freq.index, 0.0)
            .expect("set_control");
        let rms_one = render_rms(&mut world, 48_000 / 2);
        assert!(
            rms_one > 0.05,
            "the sibling instance still sounds: rms={rms_one}",
        );
        assert!(
            rms_one < rms_both * 0.75,
            "one silenced instance is quieter: both={rms_both} one={rms_one}",
        );
        assert!(
            b.params[0].last.is_none(),
            "the sibling's slot is untouched"
        );
    }
}
