//! Bevy + plyphon DSP runtime for gantz.
//!
//! [`PlyphonPlugin`] owns the dsp engine: at startup it opens a cpal output
//! stream (its own audio thread, running [`plyphon::World::fill_at`] so the engine
//! clock is anchored to the host) and keeps the [`plyphon::Controller`] +
//! [`plyphon::Nrt`] handles. Each update, after [`bevy_gantz::VmSet`], it derives
//! one synthdef per `~bus`-cut *region* of each open head's DSP subgraph (rooted
//! at its `~out` outputs and `~scopeout` monitors) and reconciles them with the
//! running synths via the [`gantz_plyphon::Backend`] seam whenever the head's
//! committed graph changes: unchanged regions keep their synths (and their unit
//! state - oscillator phase, delay lines), changed ones are crossfade-replaced.
//!
//! The bridge runs both ways: control values drive dsp params via `set_control`,
//! and each `~scopeout` monitor's scope stream is drained here into the node's
//! ring-buffer state (so the control world can scope a dsp signal).
//!
//! Mixing across heads is free: every head's `~out` synth writes to output bus 0,
//! and plyphon sums all synths on that bus.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::system::NonSendMut;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use gantz_ca as ca;
use gantz_core::node::graph::Graph;
use gantz_plyphon::{
    AddAction, Backend, Embedded, GainRef, ROOT_GROUP_ID, RegionDerived, ToNodeDsp,
    derive_synthdefs, flatten::AsNamedRef, flatten_from_registry, structural_sig,
};
use plyphon::{Controller, InputRef, Nrt, Options, StreamConsumer, World, engine};
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

use bevy_gantz::head::{HeadRef, HeadVms, OpenHead, WorkingGraph};
use bevy_gantz::{DspConfig, DspStatus, EntrypointSet, EvalEpoch, Registry, VmSet};

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
/// [`ToNodeDsp`]). Add it *after* [`bevy_gantz::GantzPlugin`].
///
/// To use custom UGens, register them with [`with_units`](Self::with_units). A
/// node's [`NodeDsp::ugens`](gantz_plyphon::NodeDsp::ugens) can then name them in a
/// [`UnitSpec`](plyphon::UnitSpec).
pub struct PlyphonPlugin<N> {
    unit_registrars: Vec<UnitRegistrar>,
    _marker: std::marker::PhantomData<N>,
}

impl<N> Default for PlyphonPlugin<N> {
    fn default() -> Self {
        Self {
            unit_registrars: Vec::new(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<N> PlyphonPlugin<N> {
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
    /// PlyphonPlugin::<N>::new().with_units(|reg| {
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

impl<N> Plugin for PlyphonPlugin<N>
where
    N: 'static + ToNodeDsp + gantz_core::Node + AsNamedRef + Clone + Send + Sync,
{
    fn build(&self, app: &mut App) {
        // The shared monotonic epoch (set by `GantzPlugin`, added before this) is
        // the dsp clock's time base. The cpal callback anchors the engine clock
        // to it via `fill_at`, matching the firing times queued into `%args`.
        let epoch = *app.world().resource::<EvalEpoch>();
        // DSP settings (the Settings -> DSP tab) + the status it reports.
        app.init_resource::<DspConfig>();
        let mut head_synths = HeadSynths::default();
        let status = match build_dsp_engine(epoch, &self.unit_registrars) {
            Some(engine) => {
                let status = DspStatus {
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
                DspStatus::default()
            }
        };
        app.insert_resource(status);
        // `.after(EntrypointSet)`: run once the `tick!`/`update!` drivers' triggered
        // evaluations have flushed, so the control values they queue are visible to
        // the param drain below in the same frame. `HeadSynths` is a NonSend resource:
        // it holds each `~scopeout`'s scope `StreamConsumer` (a `!Sync` SPSC handle).
        app.insert_non_send(head_synths)
            .add_systems(Update, drive_synths::<N>.after(VmSet).after(EntrypointSet));
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

/// Per-head synth bookkeeping (which committed graph produced the currently
/// installed synthdef and running synth, so we only re-derive on change), plus the
/// allocator of global scope-stream indices for `~scopeout`s. A NonSend resource - a
/// `~scopeout`'s scope `StreamConsumer` is `Send` but not `Sync`.
#[derive(Default)]
struct HeadSynths {
    heads: HashMap<Entity, HeadRegions>,
    scope_alloc: ScopeAlloc,
    bus_alloc: BusAlloc,
    /// Crossfade-retired synths still ramping their gains to zero, freed once
    /// their deadline passes (oldest first - entries are pushed in replacement
    /// order). Their scope streams ride along: a stream index must not be
    /// re-cued while the old `ScopeOut` (bufnum baked into its def) can still
    /// write it.
    fading: Vec<FadingSynth>,
}

/// One open head's running synths: one per boundary-cut region, in region-DAG
/// topological order (bus writers before their readers - also their node-tree
/// order). Empty when the head's graph has no dsp sink.
struct HeadRegions {
    graph: ca::GraphAddr,
    regions: Vec<RegionSynth>,
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
    allocated: HashMap<(Entity, Vec<usize>), Run>,
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
        key: (Entity, Vec<usize>),
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
    fn release(&mut self, key: &(Entity, Vec<usize>), now: Instant) {
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

/// The installed synthdef + running synth for one region of a head.
struct RegionSynth {
    /// The region's identity across re-derives (`RegionDerived::key`), the
    /// driver's match for the keep/replace decision.
    key: u64,
    def_name: String,
    node_id: i32,
    /// Structural signature of the running synth's def (excludes param values) -
    /// unchanged across a structural-graph edit means the synth need not respawn.
    sig: u64,
    /// One slot per control param, binding a dsp node's state value to its synth
    /// param index, with the last value pushed via `set_control`.
    params: Vec<ParamSlot>,
    /// One slot per `~scopeout`: the cued scope stream whose samples the driver drains into
    /// the node's ring state each frame.
    scopes: Vec<ScopeSlot>,
    /// The def's driver-owned fade gains (one per sink), used to fade the synth
    /// in on spawn and out across a crossfaded replacement.
    gains: Vec<GainRef>,
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
fn drive_synths<N>(
    registry: Res<Registry<N>>,
    dsp: Option<NonSendMut<DspEngine>>,
    dsp_config: Res<DspConfig>,
    mut enabled_applied: Local<Option<bool>>,
    state: NonSendMut<HeadSynths>,
    mut vms: NonSendMut<HeadVms>,
    heads: Query<(Entity, &HeadRef, &WorkingGraph<N>), With<OpenHead>>,
) where
    N: 'static + ToNodeDsp + gantz_core::Node + AsNamedRef + Clone + Send + Sync,
{
    let Some(dsp) = dsp else {
        return;
    };
    let dsp = dsp.into_inner();
    let state = state.into_inner();

    // Apply the enable/mute toggle by pausing/playing the output stream on change.
    if *enabled_applied != Some(dsp_config.enabled) {
        let result = if dsp_config.enabled {
            dsp.stream.play()
        } else {
            dsp.stream.pause()
        };
        if let Err(e) = result {
            log::error!("bevy_gantz_plyphon: audio stream play/pause failed: {e}");
        }
        *enabled_applied = Some(dsp_config.enabled);
    }

    // Tick NRT cleanup off the audio thread (drops freed synths, surfaces events).
    dsp.nrt.process();
    while dsp.nrt.poll().is_some() {}

    let out_channels = dsp.out_channels;
    let sample_rate = dsp.sample_rate;
    let mut live: HashSet<Entity> = HashSet::new();

    for (entity, head_ref, wg) in heads.iter() {
        live.insert(entity);
        let Some(graph_ca) = registry.head_commit(&head_ref.0).map(|c| c.graph) else {
            continue;
        };

        // Structural sync: only when the committed graph changed. Flattening
        // first splices any nested graphs (resolved through the registry) into
        // one flat graph whose nodes carry their original nested paths. On a
        // flatten error (a ref cycle or an unresolvable ref, both forbidden
        // upstream): keep the previous synths, parking the head at this graph
        // address so the error logs once per commit rather than every frame.
        if state.heads.get(&entity).map(|h| h.graph) != Some(graph_ca) {
            match flatten_from_registry(&wg.0, &registry) {
                Ok(flat) => structural_sync(
                    &mut dsp.controller,
                    state,
                    entity,
                    graph_ca,
                    &flat,
                    out_channels,
                    sample_rate,
                ),
                Err(e) => {
                    log::error!(
                        "bevy_gantz_plyphon: flattening nested graphs failed ({e:?}), \
                         keeping the previous synths"
                    );
                    park_head(state, entity, graph_ca);
                }
            }
        }

        // Param sync: drain each param's queued control updates and schedule
        // them ahead of the dsp clock. direct (untimestamped) value edits apply
        // immediately.
        let (Some(head), Some(vm)) = (state.heads.get_mut(&entity), vms.get_mut(&entity)) else {
            continue;
        };
        for synth in &mut head.regions {
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
                        let when = osc(t + dsp_config.sched_lead.as_secs_f64());
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
            for synth in head.regions {
                let def_name = synth.def_name.clone();
                fade_out(&mut dsp.controller, state, e, synth);
                let _ = Embedded::new(&mut dsp.controller).free_synthdef(&def_name);
            }
        }
        state.bus_alloc.release_head(e, Instant::now());
    }

    // Sweep crossfade-retired synths, freeing those whose fade has died away,
    // and return quarantined bus runs whose grace has passed.
    for f in expire_fades(&mut state.fading, Instant::now()) {
        let _ = Embedded::new(&mut dsp.controller).free_node(f.node_id);
        free_scopes(&mut dsp.controller, &mut state.scope_alloc, f.scopes);
    }
    state.bus_alloc.sweep(Instant::now());
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
/// signature is computed on the *unpatched* def - `ScopeOut` bufnums, bus
/// channels and fade defaults are all placeholders at that point - so a
/// re-derive of the same graph never spuriously respawns.
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
                HeadRegions {
                    graph: graph_ca,
                    regions: Vec::new(),
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
fn structural_sync<N>(
    controller: &mut Controller,
    state: &mut HeadSynths,
    entity: Entity,
    graph_ca: ca::GraphAddr,
    graph: &Graph<N>,
    out_channels: usize,
    sample_rate: f64,
) where
    N: ToNodeDsp,
{
    let now = Instant::now();
    let prefix = format!("gantz-head-{}", entity.index());
    let derived = match derive_synthdefs(graph, out_channels, &prefix) {
        Ok(regions) => regions,
        // No dsp sink to root any synthdef at: fade every region out. Freeing
        // the defs now is safe - a fading synth holds its own compiled copy.
        Err(gantz_plyphon::DeriveError::NoSink) => {
            if let Some(head) = state.heads.remove(&entity) {
                for synth in head.regions {
                    let def_name = synth.def_name.clone();
                    fade_out(controller, state, entity, synth);
                    let _ = Embedded::new(controller).free_synthdef(&def_name);
                }
            }
            state.bus_alloc.release_head(entity, now);
            // An empty entry parks the head at this graph address so sinkless
            // graphs don't re-derive every frame.
            state.heads.insert(
                entity,
                HeadRegions {
                    graph: graph_ca,
                    regions: Vec::new(),
                },
            );
            return;
        }
        // A bus cycle has no runnable writer-before-reader order: keep the
        // previous synths, recording the graph address so the error logs once
        // per commit rather than every frame.
        Err(gantz_plyphon::DeriveError::BusCycle) => {
            log::error!(
                "bevy_gantz_plyphon: `~bus` cycle between regions, keeping the previous synths"
            );
            park_head(state, entity, graph_ca);
            return;
        }
    };

    // Release bus runs whose boundary nodes are gone from the derivation.
    let live_buses: HashSet<Vec<usize>> = derived
        .iter()
        .flat_map(|r| r.bus_writes.iter().chain(&r.bus_reads))
        .map(|b| b.node_path.clone())
        .collect();
    let dead_buses: Vec<_> = state
        .bus_alloc
        .allocated
        .keys()
        .filter(|(e, path)| *e == entity && !live_buses.contains(path))
        .cloned()
        .collect();
    for key in dead_buses {
        state.bus_alloc.release(&key, now);
    }

    // Plan each region: keep the running synth when key + sig match, else spawn
    // a replacement (fading any predecessor once the replacement is up).
    enum Plan {
        Keep(RegionSynth),
        Spawn(RegionDerived, u64, Option<RegionSynth>),
    }
    let mut prev = state
        .heads
        .remove(&entity)
        .map(|h| h.regions)
        .unwrap_or_default();
    let mut plans: Vec<Plan> = Vec::with_capacity(derived.len());
    for r in derived {
        let sig = structural_sig(&r.derived.def);
        match prev.iter().position(|p| p.key == r.key) {
            Some(ix) => {
                let mut p = prev.remove(ix);
                if p.sig == sig {
                    // Structure unchanged (e.g. a non-dsp edit, or a ring-size
                    // edit that is not in the def): keep the synth + its param
                    // slots + its cued scope streams. Refresh each tap's ring
                    // `size` in case a size edit changed it.
                    for m in &r.derived.monitors {
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
                    plans.push(Plan::Spawn(r, sig, Some(p)));
                }
            }
            None => plans.push(Plan::Spawn(r, sig, None)),
        }
    }
    // Regions that disappeared entirely: fade out + free their defs.
    for old in prev {
        let def_name = old.def_name.clone();
        fade_out(controller, state, entity, old);
        let _ = Embedded::new(controller).free_synthdef(&def_name);
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

    let mut regions: Vec<RegionSynth> = Vec::with_capacity(plans.len());
    for (plan, anchor) in plans.into_iter().zip(anchors) {
        match plan {
            Plan::Keep(k) => regions.push(k),
            Plan::Spawn(r, sig, old) => {
                match spawn_region(controller, state, entity, r, sig, sample_rate, anchor) {
                    Some(synth) => {
                        // The replacement is live: fade the old out (the gain
                        // overlap is the crossfade). The def name was just
                        // re-installed, so the old def is already retired.
                        if let Some(old) = old {
                            fade_out(controller, state, entity, old);
                        }
                        regions.push(synth);
                    }
                    // Keep the old synth playing untouched on failure -
                    // strictly better than the silence a teardown would leave.
                    None => regions.extend(old),
                }
            }
        }
    }
    state.heads.insert(
        entity,
        HeadRegions {
            graph: graph_ca,
            regions,
        },
    );
}

/// Cue scope streams and patch every placeholder (`ScopeOut` bufnums, bus
/// channels from [`BusAlloc`], fade-gain defaults to `0.0` so the synth spawns
/// silent), then install and spawn one region's def - `Before` the given anchor
/// when present, else at the root group's tail. Unbound fade gains are ramped
/// straight to unity (bound params re-send from node state via the same-frame
/// param sync). Returns `None` on failure, having cleaned up after itself.
fn spawn_region(
    controller: &mut Controller,
    state: &mut HeadSynths,
    entity: Entity,
    region: RegionDerived,
    sig: u64,
    sample_rate: f64,
    anchor: Option<i32>,
) -> Option<RegionSynth> {
    let now = Instant::now();
    let RegionDerived {
        key,
        derived,
        bus_writes,
        bus_reads,
    } = region;
    let gantz_plyphon::Derived {
        mut def,
        params,
        monitors,
        gains,
    } = derived;

    // Patch the bus placeholders from the per-bus-node allocations.
    for binding in bus_writes.iter().chain(&bus_reads) {
        let bus_key = (entity, binding.node_path.clone());
        let Some(run) = state.bus_alloc.get_or_alloc(bus_key, binding.channels, now) else {
            log::error!("bevy_gantz_plyphon: private audio buses exhausted; region not spawned");
            return None;
        };
        def.units[binding.unit].inputs[0] = InputRef::Constant(run.start as f32);
    }

    // Cue a scope stream per `~scopeout`, patching its `ScopeOut` bufnum.
    let mut scopes = Vec::new();
    for m in &monitors {
        let index = state.scope_alloc.alloc();
        def.units[m.scope_unit].inputs[0] = InputRef::Constant(index as f32);
        let channels = m.channels.max(1);
        match controller.cue_scope(index, channels, sample_rate, CHUNK_FRAMES, NUM_CHUNKS) {
            Ok(consumer) => scopes.push(ScopeSlot {
                node_path: m.node_path.clone(),
                size: m.size,
                channels,
                index,
                consumer,
            }),
            Err(e) => {
                log::error!("bevy_gantz_plyphon: cue_scope failed: {e:?}");
                state.scope_alloc.free(index);
            }
        }
    }
    // Spawn silent; the fades ramp in below / via the same-frame param sync.
    for g in &gains {
        def.params[g.index].default = 0.0;
    }

    let def_name = def.name.clone();
    let mut backend = Embedded::new(controller);
    if let Err(e) = backend.install_synthdef(def) {
        log::error!("bevy_gantz_plyphon: synthdef install failed: {e:?}");
        drop(backend);
        free_scopes(controller, &mut state.scope_alloc, scopes);
        return None;
    }
    let (target, action) = match anchor {
        Some(node) => (node, AddAction::Before),
        None => (ROOT_GROUP_ID, AddAction::Tail),
    };
    match backend.spawn(&def_name, target, action) {
        Ok(node_id) => {
            // A gain with no param slot (no node state feeds it - every fade
            // gain) would stay stuck at the patched 0.0 default. Ramp it
            // straight to unity instead.
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
            Some(RegionSynth {
                key,
                def_name,
                node_id,
                sig,
                params,
                scopes,
                gains,
            })
        }
        Err(e) => {
            log::error!("bevy_gantz_plyphon: synth spawn failed: {e:?}");
            let _ = backend.free_synthdef(&def_name);
            drop(backend);
            free_scopes(controller, &mut state.scope_alloc, scopes);
            None
        }
    }
}

/// Begin fading `synth` out: ramp every gain to zero (each through its own lag)
/// and queue the synth - with its cued scope streams - for a deferred free once
/// the slowest ramp has died away. A synth with no gains (e.g. monitor-only) has
/// no audible output to de-click and is freed immediately.
fn fade_out(
    controller: &mut Controller,
    state: &mut HeadSynths,
    entity: Entity,
    synth: RegionSynth,
) {
    if synth.gains.is_empty() {
        let _ = Embedded::new(controller).free_node(synth.node_id);
        free_scopes(controller, &mut state.scope_alloc, synth.scopes);
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
    state.fading.push(FadingSynth {
        entity,
        node_id: synth.node_id,
        deadline: Instant::now() + FADE_GRACE.max(Duration::from_secs_f32(max_lag)),
        scopes: synth.scopes,
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
            .get_or_alloc((e[0], vec![1]), 2, base)
            .expect("2 channels");
        assert_eq!(a, Run { start: 4, len: 2 });
        let b = alloc
            .get_or_alloc((e[0], vec![2]), 3, base)
            .expect("3 channels");
        assert_eq!(b, Run { start: 6, len: 3 });
        // Same key + width: the same run (an unchanged region's baked channel
        // stays valid).
        assert_eq!(alloc.get_or_alloc((e[0], vec![1]), 2, base), Some(a));
        // A width change re-allocates; the old run is quarantined, not reused.
        let a2 = alloc
            .get_or_alloc((e[0], vec![1]), 4, base)
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
        let a = alloc.get_or_alloc((e[0], vec![1]), 2, base).expect("a");
        let b = alloc.get_or_alloc((e[0], vec![2]), 2, base).expect("b");
        assert_eq!((a.start, b.start), (0, 2));
        // Exhausted while both are live.
        assert_eq!(alloc.get_or_alloc((e[0], vec![3]), 1, base), None);
        // Released runs stay quarantined until the deadline passes...
        alloc.release_head(e[0], base);
        alloc.sweep(base);
        assert_eq!(alloc.get_or_alloc((e[0], vec![3]), 1, base), None);
        // ...then coalesce back into one allocatable 4-wide run.
        alloc.sweep(base + FADE_GRACE * 2);
        assert_eq!(
            alloc.get_or_alloc((e[0], vec![3]), 4, base),
            Some(Run { start: 0, len: 4 }),
        );
    }
}
