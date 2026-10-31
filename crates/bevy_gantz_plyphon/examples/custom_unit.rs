//! A downstream custom UGen and custom DSP node, end to end through the bevy
//! and plyphon runtime.
//!
//! Run with `cargo run --example custom_unit -p bevy_gantz_plyphon` to hear a 220 Hz
//! saw played by a custom `Saw` UGen wired into a `~saw -> ~out` graph. It shows the
//! whole downstream path:
//!
//! 1. a custom plyphon [`Unit`], `Saw`, and its [`UnitDef`], `SawCtor`.
//! 2. a custom gantz DSP node, `SawNode`, whose [`NodeDsp::ugens`] emits that unit.
//! 3. a tiny node codec over the set, `SawNode` and the reused `~out`, through
//!    which the runtime's reified-graph cache serves the graph.
//! 4. a headless bevy app that registers the unit via
//!    [`PlyphonPlugin::with_units`], builds the graph, and plays it.
//!
//! `Saw`'s frequency is baked in for brevity. See `gantz_plyphon`'s `~sinosc` for the
//! settable-control-param pattern. That is a `push_param` plus node VM state the
//! driver reads.

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use std::time::Duration;

use bevy_gantz::{GantzPlugin, Registry, head, timestamp};
use bevy_gantz_egui::{GraphCache, NodeCodecRes};
use bevy_gantz_plyphon::PlyphonPlugin;
use bytemuck::{Pod, Zeroable};
use gantz_ca::Head;
use gantz_nodedef::{ExprArgs, Meta, NodeDef};
use gantz_plyphon::{DspBuilder, NodeDsp, Signal};
use plyphon::synthdef::{InputRef, UnitSpec};
use plyphon::{
    BuildContext, BuildError, BuiltUnit, DoneAction, ProcessCtx, Rate, Unit, UnitDef, unit_spec,
};

// 1. A custom plyphon UGen, a band-unlimited saw oscillator.

/// The unit's per-instance state. A phase accumulator and its per-sample
/// increment. Units are `#[repr(C)]` and `Pod`, since their bytes live in the
/// engine's rt-pool.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Saw {
    phase: f32,
    inc: f32,
}

/// Output amplitude. A naive band-unlimited saw is harsh, so keep it gentle.
const AMP: f32 = 0.2;

impl Unit for Saw {
    fn process(&mut self, ctx: &mut ProcessCtx<'_>) -> DoneAction {
        for o in ctx.outs.audio(0).iter_mut() {
            *o = (self.phase * 2.0 - 1.0) * AMP; // 0..1 ramp to -1..1 saw, scaled
            self.phase += self.inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
        DoneAction::Nothing
    }
}

/// Builds a [`Saw`] off the audio thread. Reads its frequency from input 0, a
/// baked constant, and the engine sample rate to compute the phase increment.
struct SawCtor;

impl UnitDef for SawCtor {
    fn build(&self, ctx: &BuildContext<'_>) -> Result<BuiltUnit, BuildError> {
        let freq = ctx.const_input(0).unwrap_or(220.0);
        let inc = (freq as f64 / ctx.audio.sample_rate) as f32;
        Ok(unit_spec(Saw { phase: 0.0, inc }))
    }
}

// 2. A custom gantz DSP node emitting the `Saw` unit.

/// A saw-oscillator DSP node. `NodeDef` defines its node type, with no view
/// beside its name. It is Steel-inert, since audio is plyphon's job.
/// `NodeDsp` emits its UGen graph.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct SawNode {
    freq: f32,
}

impl NodeDef for SawNode {
    const TAG: &'static str = "SawNode";

    fn meta(&self) -> Meta {
        Meta {
            name: "~saw".into(),
            outputs: 1,
            ..Meta::default()
        }
    }

    fn expr(&self, _: ExprArgs<'_>) -> Result<String, String> {
        // Steel-inert. A placeholder output for the ignored dsp edge.
        Ok("0".into())
    }
}

impl NodeDsp for SawNode {
    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn ugens(
        &self,
        _path: &[usize],
        _inputs: &[Option<Signal>],
        b: &mut DspBuilder,
    ) -> Vec<Signal> {
        // Freq is a baked constant. See ~sinosc for a settable param.
        let unit = b.push_unit(UnitSpec::new(
            "Saw",
            Rate::Audio,
            vec![InputRef::Constant(self.freq)],
            1,
        ));
        vec![Signal::mono(InputRef::Unit { unit, output: 0 })]
    }
}

// 3. The node codec over the `~saw` source and the reused `~out` sink.

/// The value-level codec over the example's node set. The seam through which
/// the runtime's reified-graph cache serves the stored graph as typed nodes.
fn codec() -> gantz_egui::node::NodeCodec {
    gantz_egui::node::NodeCodec::new(
        gantz_format::Sugars::default(),
        [
            gantz_egui::node::dyn_def_kind(gantz_plyphon::node_def::<SawNode>()),
            gantz_egui::node::dyn_def_kind(gantz_plyphon::node_def::<gantz_plyphon::Out>()),
        ],
    )
}

// 4. The headless bevy app.

fn main() {
    App::new()
        // Headless. Tick the schedule about 60 times per second with no window.
        .add_plugins(
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 60.0,
            ))),
        )
        .add_plugins(GantzPlugin)
        // Register the custom `Saw` unit into the embedded engine at startup.
        .add_plugins(PlyphonPlugin::new().with_units(|reg| {
            reg.register("Saw", Box::new(SawCtor));
        }))
        // The typed side the DSP driver reads. This example has no
        // `GantzEguiPlugin`. It builds its graph in code and fills the cache in
        // `setup`.
        .insert_resource(NodeCodecRes(codec()))
        .init_resource::<GraphCache>()
        .add_systems(Startup, setup)
        .run();
}

/// Build `~saw -> ~out`, commit it, and open it as a head. `drive_synths` derives a
/// synthdef from the `~out` root and spawns it. The cpal stream then plays the saw.
fn setup(mut registry: ResMut<Registry>, mut cache: ResMut<GraphCache>, mut cmds: Commands) {
    // The registry stores graphs as erased data. The graph address is always
    // computed on the erased form.
    let mut dg = gantz_ca::DataGraph::default();
    let saw = dg.add_node(gantz_nodedef::node_data(&SawNode { freq: 220.0 }).unwrap());
    let out = dg.add_node(gantz_nodedef::node_data(&gantz_plyphon::Out::default()).unwrap());
    dg.add_edge(saw, out, gantz_ca::Edge::from((0, 0)));

    let graph_ca = gantz_ca::graph_addr(&dg);
    let commit = registry.commit_graph(timestamp(), None, graph_ca, move || dg);
    // Reify the committed graph through the codec so the DSP driver can read it.
    bevy_gantz_egui::refresh_cache(&registry, &mut cache, &codec());
    cmds.trigger(head::OpenEvent(Head::Commit(commit)));

    println!("Playing a custom `Saw` UGen at 220 Hz through bevy_gantz_plyphon. Ctrl-C to stop.");
}
