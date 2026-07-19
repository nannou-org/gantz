//! A downstream **custom UGen + custom DSP node**, end to end through the bevy +
//! plyphon runtime.
//!
//! Run with `cargo run --example custom_unit -p bevy_gantz_plyphon` to hear a 220 Hz
//! saw played by a custom `Saw` UGen wired into a `~saw -> ~out` graph. It shows the
//! whole downstream path:
//!
//! 1. a custom plyphon [`Unit`] (`Saw`) + its [`UnitDef`] (`SawCtor`).
//! 2. a custom gantz DSP node (`SawNode`) whose [`NodeDsp::ugens`] emits that unit.
//! 3. a tiny node codec over the set (`SawNode` + the reused `~out`), through
//!    which the runtime's reified-graph cache serves the graph.
//! 4. a headless bevy app that registers the unit via
//!    [`PlyphonPlugin::with_units`], builds the graph, and plays it.
//!
//! `Saw`'s frequency is baked in for brevity. See `gantz_plyphon`'s `~sinosc` for the
//! settable-control-param pattern (a `push_param` + node VM state the driver reads).

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use std::time::Duration;

use bevy_gantz::{GantzPlugin, Registry, head, timestamp};
use bevy_gantz_egui::{GraphCache, NodeCodecRes};
use bevy_gantz_plyphon::PlyphonPlugin;
use bytemuck::{Pod, Zeroable};
use gantz_ca::Head;
use gantz_core::Node as GantzNode;
use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, parse_expr};
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
/// - audio is plyphon's job); `NodeDsp` emits its UGen graph; `NodeUi` gives the
/// erased UI node its (minimal) rendering.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, gantz_nodetag::NodeTag)]
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

impl gantz_egui::NodeUi for SawNode {
    fn name(&self, _: &gantz_egui::Env<'_>) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("~saw")
    }

    fn ui(
        &mut self,
        _ctx: gantz_egui::NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> gantz_egui::NodeUiResponse {
        let framed = uictx.framed(|ui, _sockets| ui.label("~saw"));
        gantz_egui::NodeUiResponse::new(framed)
    }
}

// ---------------------------------------------------------------------------
// 3. The node codec: our source + the reused `~out` sink.
// ---------------------------------------------------------------------------

/// The example's `.gantz` sugar carrier (unused here beyond the codec's
/// requirement - this example never parses or exports text).
struct NodeSet;

impl gantz_format::NodeSugar for NodeSet {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![&gantz_format::CoreSugar])
    }
}

/// The value-level codec over the example's node set: the seam through which
/// the runtime's reified-graph cache serves the stored graph as typed nodes.
fn codec() -> gantz_egui::node::NodeCodec {
    gantz_egui::ui_node_codec! {
        NodeSet {
            SawNode,
            gantz_plyphon::Out,
        }
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
        .add_plugins(GantzPlugin)
        // Register the custom `Saw` unit into the embedded engine at startup.
        .add_plugins(PlyphonPlugin::new().with_units(|reg| {
            reg.register("Saw", Box::new(SawCtor));
        }))
        // The typed side the DSP driver reads. This example builds its graph
        // in code and fills the cache itself in `setup` (no `GantzEguiPlugin`).
        .insert_resource(NodeCodecRes(codec()))
        .init_resource::<GraphCache>()
        .add_systems(Startup, setup)
        .run();
}

/// Build `~saw -> ~out`, commit it, and open it as a head. `drive_synths` derives a
/// synthdef from the `~out` root and spawns it. The cpal stream then plays the saw.
fn setup(mut registry: ResMut<Registry>, mut cache: ResMut<GraphCache>, mut cmds: Commands) {
    // The registry stores graphs as erased data (the graph address is always
    // computed on the erased form).
    let mut dg = gantz_ca::DataGraph::default();
    let saw = dg.add_node(gantz_core::data::erase_node_typed(&SawNode { freq: 220.0 }).unwrap());
    let out =
        dg.add_node(gantz_core::data::erase_node_typed(&gantz_plyphon::Out::default()).unwrap());
    dg.add_edge(saw, out, gantz_ca::Edge::from((0, 0)));

    let graph_ca = gantz_ca::graph_addr(&dg);
    let commit = registry.commit_graph(timestamp(), None, graph_ca, move || dg);
    // Reify the committed graph through the codec so the DSP driver can read it.
    bevy_gantz_egui::refresh_cache(&registry, &mut cache, &codec());
    cmds.trigger(head::OpenEvent(Head::Commit(commit)));

    println!("Playing a custom `Saw` UGen at 220 Hz through bevy_gantz_plyphon. Ctrl-C to stop.");
}
