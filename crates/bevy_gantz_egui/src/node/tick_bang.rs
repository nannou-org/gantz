//! A self-driven node that fires once per configurable tick duration.
//!
//! Unlike `update!`, which bangs once per update, `tick!` owns its own time
//! accumulator fed from Bevy `Time` and fires once for *every* whole tick
//! duration elapsed since the last update (fixed-timestep catch-up). This keeps
//! the tick *count* correct even when the app updates more slowly than the tick
//! rate. Evaluation is driven by the [`drive_tick_bangs`] Bevy system rather
//! than from the node's `ui()` method, so it continues even when the graph tab
//! is not visible.

use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_time::prelude::*;
use gantz_ca::CaHash;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::visit;
use gantz_egui::widget::node_inspector::radio_option;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use steel::SteelVal;

/// The default tick duration in seconds when unconfigured.
const DEFAULT_DURATION: f64 = 1.0;

/// The smallest tick duration the inspector allows, in seconds.
const MIN_DURATION: f64 = 0.001;

/// The smallest tick rate the inspector allows, in Hz.
const MIN_RATE: f64 = 0.001;

/// The most ticks a single `tick!` node may fire in one update.
///
/// Caps fixed-timestep catch-up so a long stall (e.g. the window was hidden or
/// a breakpoint paused the app) cannot trigger an unbounded burst of
/// evaluations - any backlog beyond this many ticks is discarded.
const MAX_CATCHUP_TICKS: f64 = 64.0;

// ---------------------------------------------------------------------------
// TickBang node
// ---------------------------------------------------------------------------

/// How a [`TickBang`]'s tick interval is specified.
///
/// Stored in the user's chosen unit so it round-trips exactly - deriving Hz
/// from a stored duration (or vice versa) would accumulate float error across
/// edits.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Interval {
    /// A duration in seconds between ticks.
    Duration(f64),
    /// A rate in Hz (ticks per second).
    Rate(f64),
}

impl Interval {
    /// The effective tick duration in seconds.
    pub fn duration(self) -> f64 {
        match self {
            Interval::Duration(secs) => secs,
            Interval::Rate(hz) => 1.0 / hz,
        }
    }

    /// Whether the interval is specified as a rate (Hz) rather than a duration.
    pub fn is_rate(self) -> bool {
        matches!(self, Interval::Rate(_))
    }
}

impl Default for Interval {
    fn default() -> Self {
        Interval::Duration(DEFAULT_DURATION)
    }
}

impl PartialEq for Interval {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Interval::Duration(a), Interval::Duration(b)) => a.to_bits() == b.to_bits(),
            (Interval::Rate(a), Interval::Rate(b)) => a.to_bits() == b.to_bits(),
            _ => false,
        }
    }
}

impl Eq for Interval {}

impl Hash for Interval {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Fully qualified to disambiguate from `gantz_ca::CaHash::hash`.
        match self {
            Interval::Duration(secs) => {
                Hash::hash(&0u8, state);
                Hash::hash(&secs.to_bits(), state);
            }
            Interval::Rate(hz) => {
                Hash::hash(&1u8, state);
                Hash::hash(&hz.to_bits(), state);
            }
        }
    }
}

/// A self-driven node that fires once per configurable tick interval.
///
/// The interval is set either as a duration (seconds) or a rate (Hz). Outputs
/// the effective tick duration in seconds as `f64` on each tick. The driver
/// fires it once for every whole interval elapsed since the last update, so the
/// tick *count* stays correct even when updates are slower than the tick rate.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TickBang {
    #[serde(default)]
    interval: Interval,
}

impl TickBang {
    /// How the tick interval is specified (a duration in seconds or rate in Hz).
    pub fn interval(&self) -> Interval {
        self.interval
    }

    /// Set how the tick interval is specified (content-address affecting).
    pub fn set_interval(&mut self, interval: Interval) {
        self.interval = interval;
    }

    /// The effective tick duration in seconds.
    pub fn duration(&self) -> f64 {
        self.interval.duration()
    }
}

impl Default for TickBang {
    fn default() -> Self {
        TickBang {
            interval: Interval::default(),
        }
    }
}

impl CaHash for TickBang {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        hasher.update("gantz.tick!".as_bytes());
        // The interval - and the unit it's specified in - is part of the node's
        // identity: editing it gives the node a new address, which is how the
        // commit-on-change model persists it (only committed graphs are saved).
        match self.interval {
            Interval::Duration(secs) => {
                hasher.update(b"duration");
                CaHash::hash(&secs.to_bits(), hasher);
            }
            Interval::Rate(hz) => {
                hasher.update(b"rate");
                CaHash::hash(&hz.to_bits(), hasher);
            }
        }
    }
}

impl gantz_core::Node for TickBang {
    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // The per-tick output is the (constant) tick duration. The time
        // accumulator also lives in this node's state but is owned entirely by
        // `drive_tick_bangs`; eval reads `state` and writes it back untouched.
        // `{:?}` formats the float with a guaranteed `.`/exponent so Steel
        // parses it as a number rather than an integer.
        node::parse_expr(&format!("(begin {:?})", self.duration()))
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        node::state::init_value_if_absent(ctx.vm(), path, || SteelVal::NumV(0.0)).unwrap()
    }
}

impl gantz_egui::NodeUi for TickBang {
    fn name(&self, _: &dyn gantz_egui::Registry) -> &str {
        "tick!"
    }

    fn description(&self) -> Option<&'static str> {
        Some(
            "Self-driven clock that fires once per configurable tick interval, set \
             as a duration (seconds) or a rate (Hz). Fires once for every whole \
             interval elapsed since the last update, so the tick count stays \
             correct even when the app updates slower than the tick rate. Outputs \
             the tick duration in seconds.",
        )
    }

    fn ui(
        &mut self,
        _ctx: gantz_egui::NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> gantz_egui::NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("tick!").selectable(false)));
        gantz_egui::NodeUiResponse::new(framed)
    }

    fn inspector_rows(
        &mut self,
        _ctx: &mut gantz_egui::NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> gantz_egui::InspectorRowsResponse {
        let row_h = gantz_egui::widget::node_inspector::table_row_h(body.ui_mut());
        let mut changed = false;

        // Mode row: specify the interval as a duration (seconds) or rate (Hz).
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("mode").on_hover_text(
                    "specify the tick interval as a duration (seconds) or a rate (Hz)",
                );
            });
            row.col(|ui| {
                let mut rate = self.interval.is_rate();
                let mut switched = false;
                ui.horizontal(|ui| {
                    switched |= radio_option(ui, &mut rate, false, "dur.", "seconds between ticks");
                    switched |= radio_option(ui, &mut rate, true, "rate", "ticks per second (Hz)");
                });
                if switched {
                    // Toggle units while preserving the effective tick duration.
                    let dur = self.duration();
                    self.interval = if rate {
                        Interval::Rate(1.0 / dur)
                    } else {
                        Interval::Duration(dur)
                    };
                    changed = true;
                }
            });
        });

        // Value row: the duration (seconds) or rate (Hz), in the chosen unit.
        body.row(row_h, |mut row| {
            let is_rate = self.interval.is_rate();
            row.col(|ui| {
                if is_rate {
                    ui.label("rate").on_hover_text("ticks per second (Hz)");
                } else {
                    ui.label("dur.").on_hover_text("seconds between ticks");
                }
            });
            row.col(|ui| match &mut self.interval {
                Interval::Duration(secs) => {
                    let mut v = *secs;
                    let resp = ui.add(
                        egui::DragValue::new(&mut v)
                            .speed(0.01)
                            .range(MIN_DURATION..=f64::INFINITY)
                            .suffix(" s"),
                    );
                    if resp.changed() {
                        *secs = v.max(MIN_DURATION);
                        changed = true;
                    }
                }
                Interval::Rate(hz) => {
                    let mut v = *hz;
                    let resp = ui.add(
                        egui::DragValue::new(&mut v)
                            .speed(0.1)
                            .range(MIN_RATE..=f64::INFINITY)
                            .suffix(" Hz"),
                    );
                    if resp.changed() {
                        *hz = v.max(MIN_RATE);
                        changed = true;
                    }
                }
            });
        });

        let mut resp = gantz_egui::InspectorRowsResponse::default();
        if changed {
            resp.mark_changed();
        }
        resp
    }

    fn socket_doc(
        &self,
        _: &dyn gantz_egui::Registry,
        kind: gantz_egui::SocketKind,
        _ix: usize,
    ) -> Option<gantz_egui::SocketDoc> {
        match kind {
            gantz_egui::SocketKind::Output => Some(
                gantz_egui::SocketDoc::ty("number")
                    .with_description("tick duration in seconds; emitted once per elapsed tick"),
            ),
            gantz_egui::SocketKind::Input => None,
        }
    }
}

// ---------------------------------------------------------------------------
// ToTickBang trait
// ---------------------------------------------------------------------------

/// Trait for types that may contain a [`TickBang`] node.
///
/// Implement this for your top-level node wrapper so that the
/// [`drive_tick_bangs`] system can discover `tick!` nodes.
pub trait ToTickBang {
    fn to_tick_bang(&self) -> Option<&TickBang>;
}

impl ToTickBang for TickBang {
    fn to_tick_bang(&self) -> Option<&TickBang> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// TickBangCollector
// ---------------------------------------------------------------------------

/// Collects the path and configured duration of every [`TickBang`] node found
/// during graph traversal.
struct TickBangCollector {
    pub ticks: Vec<(Vec<usize>, f64)>,
}

impl<N: ToTickBang> visit::TypedVisitor<N> for TickBangCollector {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &N) {
        if let Some(tick) = node.to_tick_bang() {
            self.ticks.push((ctx.path().to_vec(), tick.duration()));
        }
    }
}

// ---------------------------------------------------------------------------
// Entrypoints
// ---------------------------------------------------------------------------

/// Return one push entrypoint per `TickBang` node in the graph.
///
/// Unlike `update!` - whose nodes all fire together every update and so share a
/// single multi-source entrypoint - `tick!` nodes fire independently (each on
/// its own duration), so each gets its own single-source entrypoint that the
/// [`drive_tick_bangs`] driver can trigger the right number of times.
pub fn entrypoints<N>(
    get_node: node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<N>,
) -> Vec<gantz_core::compile::Entrypoint>
where
    N: gantz_core::Node + ToTickBang,
{
    let mut collector = TickBangCollector { ticks: vec![] };
    gantz_core::graph::visit_typed(get_node, graph, &[], &mut collector);
    collector
        .ticks
        .into_iter()
        .map(|(path, _dur)| {
            let source = gantz_core::compile::entrypoint::push_source(path, 1);
            gantz_core::compile::entrypoint::from_sources([source])
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Bevy system
// ---------------------------------------------------------------------------

/// Drives `tick!` nodes every update, independent of GUI visibility.
///
/// For each open head and each `tick!` node, advances the node's time
/// accumulator by the update delta time and triggers one push evaluation for
/// every whole tick duration elapsed (capped by `MAX_CATCHUP_TICKS`).
pub fn drive_tick_bangs<N>(
    time: Res<Time>,
    registry: Res<crate::Registry<N>>,
    builtins: Res<bevy_gantz::BuiltinNodes<N>>,
    demos: Res<crate::Demos>,
    mut vms: NonSendMut<bevy_gantz::head::HeadVms>,
    heads: Query<(Entity, &bevy_gantz::head::WorkingGraph<N>), With<bevy_gantz::head::OpenHead>>,
    mut cmds: Commands,
) where
    N: gantz_core::Node + ToTickBang + Send + Sync,
{
    let dt = time.delta_secs_f64();

    for (entity, wg) in heads.iter() {
        let node_reg = crate::registry_ref(&registry, &builtins, &demos);
        let get_node = |ca: &gantz_ca::ContentAddr| node_reg.node(ca);

        // Collect all TickBang paths + durations.
        let mut collector = TickBangCollector { ticks: vec![] };
        gantz_core::graph::visit_typed(&get_node, &**wg, &[], &mut collector);

        if collector.ticks.is_empty() {
            continue;
        }

        let Some(vm) = vms.get_mut(&entity) else {
            continue;
        };

        for (path, dur) in &collector.ticks {
            // Defensive: the inspector clamps to `MIN_DURATION`, but never
            // divide by a non-positive duration.
            if !(*dur > 0.0) {
                continue;
            }

            // Advance this node's accumulator and count whole ticks elapsed,
            // capping catch-up so a long stall can't burst.
            let mut acc = node::state::extract::<f64>(vm, path)
                .ok()
                .flatten()
                .unwrap_or(0.0);
            acc += dt;
            let full = (acc / dur).floor();
            let n = full.min(MAX_CATCHUP_TICKS) as u32;
            // Subtract the full elapsed so the remainder is < dur; any backlog
            // beyond the cap is dropped rather than carried forward.
            acc -= full * dur;
            if let Err(e) = node::state::update_value(vm, path, SteelVal::NumV(acc)) {
                bevy_log::error!("tick! state update failed: {e}");
            }

            // Trigger one eval per elapsed tick.
            for _ in 0..n {
                let source = gantz_core::compile::entrypoint::push_source(path.clone(), 1);
                let entrypoint = gantz_core::compile::entrypoint::from_sources([source]);
                cmds.trigger(bevy_gantz::vm::EvalEntryEvent {
                    head: entity,
                    entrypoint,
                });
            }
        }
    }
}
