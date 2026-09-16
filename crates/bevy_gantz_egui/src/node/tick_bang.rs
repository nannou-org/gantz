//! A self-driven node that fires once per configurable tick duration.
//!
//! `update!` bangs once per update. `tick!` owns its own time accumulator fed
//! from Bevy `Time` and fires once for every whole tick duration elapsed since
//! the last update. This keeps the tick count correct even when the app
//! updates more slowly than the tick rate. The [`drive_tick_bangs`] Bevy
//! system drives evaluation, not the node's `ui()` method, so it continues
//! when the graph tab is not visible.

use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_time::prelude::*;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::visit;
use gantz_egui::node::DynNode;
use gantz_egui::widget::node_inspector::radio_option;
use gantz_format::{Datum, FormatError, SugarArgs, node_datum};
use gantz_nodetag::NodeTag;
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
/// Caps fixed-timestep catch-up so a long stall cannot trigger an unbounded
/// burst of evaluations. Any backlog beyond this many ticks is discarded.
const MAX_CATCHUP_TICKS: f64 = 64.0;

/// How a [`TickBang`]'s tick interval is specified.
///
/// Stored in the user's chosen unit so it round-trips exactly. Deriving Hz
/// from a stored duration would accumulate float error across edits.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Interval {
    /// A duration in seconds between ticks.
    Duration(f64),
    /// A rate in ticks per second (Hz).
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

/// Read a `(tick-bang [#:duration secs | #:rate hz])` form into a [`TickBang`]'s
/// `interval` enum field. `#:duration` and `#:rate` are mutually exclusive.
/// Neither given yields the default duration. Dispatched by
/// [`crate::sugar::BevySugar`].
pub(crate) fn read_sugar(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let duration = args.keyword_f64("duration")?;
    let rate = args.keyword_f64("rate")?;
    let fields = match (duration, rate) {
        (Some(_), Some(_)) => {
            return Err(FormatError::malformed(
                "tick-bang: specify #:duration or #:rate, not both",
            ));
        }
        (Some(secs), None) => vec![("interval", interval_datum("Duration", secs))],
        (None, Some(hz)) => vec![("interval", interval_datum("Rate", hz))],
        (None, None) => vec![],
    };
    Ok(node_datum("TickBang", fields))
}

/// Write a [`TickBang`] as a bare `tick-bang` for the default duration, else as
/// `(tick-bang #:duration secs)` or `(tick-bang #:rate hz)` per its unit.
pub(crate) fn write_sugar(node: &Datum) -> String {
    match interval_of(node) {
        Some(("Rate", hz)) => format!("(tick-bang #:rate {hz})"),
        Some(("Duration", secs)) if secs != DEFAULT_DURATION => {
            format!("(tick-bang #:duration {secs})")
        }
        _ => "tick-bang".to_string(),
    }
}

/// The externally-tagged `interval` enum datum for a variant, for example
/// `(("Rate" hz))`.
fn interval_datum(variant: &str, value: f64) -> Datum {
    Datum::Map(vec![(variant.to_string(), Datum::F64(value))])
}

/// Read a [`TickBang`]'s `interval` field back as `(variant, value)`.
fn interval_of(node: &Datum) -> Option<(&str, f64)> {
    match node.get("interval")? {
        Datum::Map(entries) => {
            let (variant, value) = entries.first()?;
            Some((variant.as_str(), value.as_f64()?))
        }
        _ => None,
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
/// The interval is set as a duration in seconds or a rate in Hz. Outputs the
/// effective tick duration in seconds as `f64` on each tick. The driver fires
/// it once for every whole interval elapsed since the last update, so the
/// tick count stays correct even when updates are slower than the tick rate.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, NodeTag)]
pub struct TickBang {
    #[serde(default, skip_serializing_if = "is_default_interval")]
    interval: Interval,
}

fn is_default_interval(interval: &Interval) -> bool {
    *interval == Interval::default()
}

impl TickBang {
    /// How the tick interval is specified. Either a duration in seconds or a
    /// rate in Hz.
    pub fn interval(&self) -> Interval {
        self.interval
    }

    /// Set how the tick interval is specified. This affects the content address.
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

impl gantz_core::Node for TickBang {
    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // The per-tick output is the constant tick duration. The time
        // accumulator also lives in this node's state but `drive_tick_bangs`
        // owns it. Eval reads `state` and writes it back untouched. `{:?}`
        // formats the float with a `.` or exponent so Steel parses it as a
        // number rather than an integer.
        node::parse_expr(&format!("(begin {:?})", self.duration()))
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        node::state::init_value_if_absent(ctx.vm(), path, || SteelVal::NumV(0.0)).unwrap()
    }
}

impl gantz_egui::NodeUi for TickBang {
    fn name(&self, _: &gantz_egui::Env<'_>) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("tick!")
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
        _: &gantz_egui::Env<'_>,
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

/// Collects the path and configured duration of every [`TickBang`] node in
/// the graph, found by [`Any`](std::any::Any) downcast of the erased UI node.
struct TickBangCollector {
    pub ticks: Vec<(Vec<usize>, f64)>,
}

impl visit::TypedVisitor<DynNode> for TickBangCollector {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &DynNode) {
        let n: &dyn gantz_core::Node = &**node;
        if let Some(tick) = (n as &dyn std::any::Any).downcast_ref::<TickBang>() {
            self.ticks.push((ctx.path().to_vec(), tick.duration()));
        }
    }
}

/// Return one push entrypoint per `TickBang` node in the graph.
///
/// `update!` nodes all fire together every update and share a single
/// multi-source entrypoint. `tick!` nodes fire independently, each on its own
/// duration, so each gets its own single-source entrypoint that
/// [`drive_tick_bangs`] can trigger the right number of times.
pub fn entrypoints(
    get_node: node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<DynNode>,
) -> Vec<gantz_core::compile::Entrypoint> {
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

/// Drives `tick!` nodes every update, independent of GUI visibility.
///
/// For each open head and each `tick!` node, advances the node's time
/// accumulator by the update delta time and triggers one push evaluation for
/// every whole tick duration elapsed. Catch-up is capped by
/// `MAX_CATCHUP_TICKS`.
pub fn drive_tick_bangs(
    time: Res<Time>,
    epoch: Res<bevy_gantz::EvalEpoch>,
    registry: Res<crate::Registry>,
    cache: Res<crate::GraphCache>,
    builtins: Res<crate::BuiltinNodes>,
    mut vms: NonSendMut<bevy_gantz::head::HeadVms>,
    heads: Query<(Entity, &bevy_gantz::head::HeadRef), With<bevy_gantz::head::OpenHead>>,
    mut cmds: Commands,
) {
    let dt = time.delta_secs_f64();
    // The frame's monotonic now. Each tick's exact firing time derives from it.
    let now = epoch.now_secs();

    for (entity, head_ref) in heads.iter() {
        // The head's committed graph, read from the reified cache. It equals
        // the working graph, see `bevy_gantz::head::WorkingGraph`.
        let Some(graph_ca) = registry.head_commit(&head_ref.0).map(|c| c.graph) else {
            continue;
        };
        let Some(graph) = cache.get(&graph_ca) else {
            continue;
        };
        let get_node =
            |ca: &gantz_ca::ContentAddr| crate::lookup_node(&cache, &builtins.instances, ca);

        let mut collector = TickBangCollector { ticks: vec![] };
        gantz_core::graph::visit_typed(&get_node, graph, &[], &mut collector);

        if collector.ticks.is_empty() {
            continue;
        }

        let Some(vm) = vms.get_mut(&entity) else {
            continue;
        };

        for (path, dur) in &collector.ticks {
            // The inspector clamps to `MIN_DURATION`, but never divide by a
            // non-positive duration.
            if !(*dur > 0.0) {
                continue;
            }

            // Advance this node's accumulator and count whole ticks elapsed.
            // Cap catch-up so a long stall cannot burst.
            let mut acc = node::state::extract::<f64>(vm, path)
                .ok()
                .flatten()
                .unwrap_or(0.0);
            acc += dt;
            let full = (acc / dur).floor();
            let n = full.min(MAX_CATCHUP_TICKS) as u32;
            // Subtract the full elapsed so the remainder is less than dur. Any
            // backlog beyond the cap is dropped, not carried forward. The
            // remainder `acc` is the time since the most recent tick boundary,
            // so the latest tick fired `acc` seconds ago.
            acc -= full * dur;
            if let Err(e) = node::state::update_value(vm, path, SteelVal::NumV(acc)) {
                bevy_log::error!("tick! state update failed: {e}");
            }

            // Trigger one eval per elapsed tick, oldest first. Each is stamped
            // with the exact monotonic time it should fire. The i-th most
            // recent tick fired at `now - acc - i*dur`, with i = 0 the latest.
            // Per-tick times let the dsp driver schedule them sample-accurately
            // across the interval rather than bunched at the frame boundary.
            for i in (0..n).rev() {
                let t = now - acc - i as f64 * *dur;
                let source = gantz_core::compile::entrypoint::push_source(path.clone(), 1);
                let entrypoint = gantz_core::compile::entrypoint::from_sources([source]);
                cmds.trigger(bevy_gantz::vm::EvalEntryEvent {
                    head: entity,
                    entrypoint,
                    time: Some(t),
                });
            }
        }
    }
}
