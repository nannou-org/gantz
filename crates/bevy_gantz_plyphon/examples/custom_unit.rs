//! A downstream **custom UGen + custom DSP node**, end to end through the bevy +
//! plyphon runtime.
//!
//! Run with `cargo run --example custom_unit -p bevy_gantz_plyphon` to hear a 220 Hz
//! saw played by a custom `Saw` UGen wired into a `~saw -> ~out` graph. It shows the
//! whole downstream path:
//!
//! 1. a custom plyphon [`Unit`] (`Saw`) + its [`UnitDef`] (`SawCtor`).
//! 2. a custom gantz DSP node (`SawNode`) whose [`NodeDsp::ugens`] emits that unit.
//! 3. a tiny node type `enum N` (no GUI/typetag machinery - a `match`-forwarding
//!    [`gantz_core::Node`]/[`CaHash`]/[`ToNodeDsp`] is all the runtime needs).
//! 4. a headless bevy app that registers the unit via
//!    [`PlyphonPlugin::with_units`], builds the graph, and plays it.
//!
//! `Saw`'s frequency is baked in for brevity. See `gantz_plyphon`'s `~sinosc` for the
//! settable-control-param pattern (a `push_param` + node VM state the driver reads).

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use std::time::Duration;

use bevy_gantz::{BuiltinNodes, GantzPlugin, Registry, head, timestamp};
use bevy_gantz_plyphon::PlyphonPlugin;
use bytemuck::{Pod, Zeroable};
use gantz_ca::{CaHash, ContentAddr, Hasher, Head, graph_addr};
use gantz_core::Node as GantzNode;
use gantz_core::edge::Edge;
use gantz_core::node::graph::Graph;
use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, RegCtx, parse_expr};
use gantz_plyphon::flatten::{AsNamedRef, NamedRef};
use gantz_plyphon::{DspBuilder, NodeDsp, Signal, ToNodeDsp};
use plyphon::synthdef::{InputRef, UnitSpec};
use plyphon::{
    BuildContext, BuildError, BuiltUnit, DoneAction, ProcessCtx, Rate, Unit, UnitDef, unit_spec,
};

// ---------------------------------------------------------------------------
// 1. A custom plyphon UGen: a band-unlimited saw oscillator.
// ---------------------------------------------------------------------------

/// The unit's per-instance state: a phase accumulator and its per-sample increment.
/// Units are `#[repr(C)] + Pod` - their bytes live in the engine's rt-pool.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Saw {
    phase: f32,
    inc: f32,
}

/// Output amplitude. A naive (band-unlimited) saw is bright/harsh, so keep it gentle.
const AMP: f32 = 0.2;

impl Unit for Saw {
    fn process(&mut self, ctx: &mut ProcessCtx<'_>) -> DoneAction {
        for o in ctx.outs.audio(0).iter_mut() {
            *o = (self.phase * 2.0 - 1.0) * AMP; // 0..1 ramp -> -1..1 saw, scaled
            self.phase += self.inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
        DoneAction::Nothing
    }
}

/// Builds a [`Saw`] off the audio thread: reads its frequency from input 0 (a baked
/// constant) and the engine sample rate to compute the phase increment.
struct SawCtor;

impl UnitDef for SawCtor {
    fn build(&self, ctx: &BuildContext<'_>) -> Result<BuiltUnit, BuildError> {
        let freq = ctx.const_input(0).unwrap_or(220.0);
        let inc = (freq as f64 / ctx.audio.sample_rate) as f32;
        Ok(unit_spec(Saw { phase: 0.0, inc }))
    }
}

// ---------------------------------------------------------------------------
// 2. A custom gantz DSP node emitting the `Saw` unit.
// ---------------------------------------------------------------------------

/// A saw-oscillator DSP node. `gantz_core::Node` makes it a graph node (Steel-inert
/// - audio is plyphon's job); `NodeDsp` emits its UGen graph.
#[derive(Clone, Debug)]
struct SawNode {
    freq: f32,
}

impl GantzNode for SawNode {
    fn n_outputs(&self, _: MetaCtx) -> usize {
        1
    }

    fn expr(&self, _: ExprCtx<'_, '_>) -> ExprResult {
        // Steel-inert: a placeholder output for the (ignored) dsp edge.
        parse_expr("0")
    }
}

impl CaHash for SawNode {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(b"example.saw");
        hasher.update(&self.freq.to_le_bytes());
    }
}

impl NodeDsp for SawNode {
    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn ugens(&self, _path: &[usize], _inputs: &[Signal], b: &mut DspBuilder) -> Vec<Signal> {
        // Name our custom unit. Freq is a baked constant (see ~sinosc for a param).
        let unit = b.push_unit(UnitSpec::new(
            "Saw",
            Rate::Audio,
            vec![InputRef::Constant(self.freq)],
            1,
        ));
        vec![Signal::mono(InputRef::Unit { unit, output: 0 })]
    }
}

impl ToNodeDsp for SawNode {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// 3. The graph's node type: our source + the reused `~out` sink.
// ---------------------------------------------------------------------------

/// A minimal node type. The runtime only needs `Clone + CaHash + Node + ToNodeDsp`,
/// so a `match`-forwarding enum suffices - no GUI/typetag machinery.
#[derive(Clone, Debug)]
enum N {
    Saw(SawNode),
    Out(gantz_plyphon::Out),
}

impl GantzNode for N {
    fn n_inputs(&self, ctx: MetaCtx) -> usize {
        match self {
            N::Saw(n) => n.n_inputs(ctx),
            N::Out(n) => n.n_inputs(ctx),
        }
    }

    fn n_outputs(&self, ctx: MetaCtx) -> usize {
        match self {
            N::Saw(n) => n.n_outputs(ctx),
            N::Out(n) => n.n_outputs(ctx),
        }
    }

    fn stateful(&self, ctx: MetaCtx) -> bool {
        match self {
            N::Saw(n) => n.stateful(ctx),
            N::Out(n) => n.stateful(ctx),
        }
    }

    fn register(&self, ctx: RegCtx<'_, '_>) {
        match self {
            N::Saw(n) => n.register(ctx),
            N::Out(n) => n.register(ctx),
        }
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        match self {
            N::Saw(n) => n.expr(ctx),
            N::Out(n) => n.expr(ctx),
        }
    }
}

impl CaHash for N {
    fn hash(&self, hasher: &mut Hasher) {
        match self {
            N::Saw(n) => n.hash(hasher),
            N::Out(n) => n.hash(hasher),
        }
    }
}

impl ToNodeDsp for N {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        match self {
            N::Saw(n) => Some(n),
            N::Out(n) => n.to_node_dsp(),
        }
    }
}

impl AsNamedRef for N {
    fn as_named_ref(&self) -> Option<&NamedRef> {
        // No nested-graph refs in this node set: nothing to flatten.
        None
    }
}

/// An empty builtin set: this example builds its graph in code, so no palette
/// constructors are needed - but `vm::sync` expects the resource to exist.
struct NoBuiltins;

impl gantz_core::Builtins for NoBuiltins {
    type Node = N;
    fn names(&self) -> Vec<&str> {
        vec![]
    }
    fn create(&self, _name: &str) -> Option<N> {
        None
    }
    fn instance(&self, _ca: &ContentAddr) -> Option<&N> {
        None
    }
    fn name(&self, _ca: &ContentAddr) -> Option<&str> {
        None
    }
    fn content_addr(&self, _name: &str) -> Option<ContentAddr> {
        None
    }
}

// ---------------------------------------------------------------------------
// 4. The headless bevy app.
// ---------------------------------------------------------------------------

fn main() {
    App::new()
        // Headless: just tick the schedule ~60x/s (no window/render).
        .add_plugins(
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 60.0,
            ))),
        )
        .add_plugins(GantzPlugin::<N>::default())
        // Register the custom `Saw` unit into the embedded engine at startup.
        .add_plugins(PlyphonPlugin::<N>::new().with_units(|reg| {
            reg.register("Saw", Box::new(SawCtor));
        }))
        .insert_resource(BuiltinNodes::<N>(Box::new(NoBuiltins)))
        .add_systems(Startup, setup)
        .run();
}

/// Build `~saw -> ~out`, commit it, and open it as a head. `drive_synths` derives a
/// synthdef from the `~out` root and spawns it. The cpal stream then plays the saw.
fn setup(mut registry: ResMut<Registry<N>>, mut cmds: Commands) {
    let mut g = Graph::<N>::default();
    let saw = g.add_node(N::Saw(SawNode { freq: 220.0 }));
    let out = g.add_node(N::Out(gantz_plyphon::Out::default()));
    g.add_edge(saw, out, Edge::new(0.into(), 0.into()));

    let graph_ca = graph_addr(&g);
    let commit = registry.commit_graph(timestamp(), None, graph_ca, move || g);
    cmds.trigger(head::OpenEvent(Head::Commit(commit)));

    println!("Playing a custom `Saw` UGen at 220 Hz through bevy_gantz_plyphon. Ctrl-C to stop.");
}
