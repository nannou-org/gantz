//! A self-driven node that fires once per configurable tick duration.
//!
//! `update!` bangs once per update. Each `tick!` node entity has a repeating
//! [`TickTimer`] fed from Bevy `Time`, and fires once for every whole tick
//! duration elapsed since the last update. This keeps the tick count correct
//! even when the app updates more slowly than the tick rate. The
//! [`drive_tick_bangs`] Bevy system drives evaluation, not the node's `ui()`
//! method, so it continues when the graph tab is not visible.

use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_gantz::node::PushEval;
use bevy_time::prelude::*;
use gantz_core::node::{self, EvalConf, ExprCtx, ExprResult, MetaCtx};
use gantz_egui::widget::kit;
use gantz_egui::widget::node_inspector::radio_option;
use gantz_format::{Datum, FormatError, SugarArgs, node_datum};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::time::Duration;

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
const MAX_CATCHUP_TICKS: u32 = 64;

/// How a [`TickBang`]'s tick interval is specified.
///
/// Stored in the user's chosen unit so it round-trips exactly. Deriving one
/// unit from the other would accumulate float error across edits.
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
#[derive(Clone, Component, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, NodeTag)]
#[require(TickTimer)]
pub struct TickBang {
    #[serde(default, skip_serializing_if = "is_default_interval")]
    interval: Interval,
}

/// The repeating timer of a `tick!` node entity. [`drive_tick_bangs`] keeps
/// its duration equal to the node's.
#[derive(Component, Debug)]
pub struct TickTimer(pub Timer);

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

impl Default for TickTimer {
    fn default() -> Self {
        let duration = Duration::from_secs_f64(DEFAULT_DURATION);
        TickTimer(Timer::new(duration, TimerMode::Repeating))
    }
}

impl gantz_core::Node for TickBang {
    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        // The entry fn that `drive_tick_bangs` fires once per tick.
        vec![EvalConf::All]
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // The per-tick output is the constant tick duration. `{:?}` formats
        // the float with a `.` or exponent so Steel parses it as a number
        // rather than an integer.
        node::parse_expr(&format!("(begin {:?})", self.duration()))
    }
}

impl gantz_egui::NodeUi for TickBang {
    fn name(&self, _: &gantz_egui::Env<'_>) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("tick!")
    }

    fn description(&self) -> Option<&'static str> {
        Some(
            "Bangs once per tick, with the tick duration in seconds. Set the tick \
             as a duration or a rate. A slow update fires each tick it missed.",
        )
    }

    fn ui(
        &mut self,
        _ctx: gantz_egui::NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> gantz_egui::NodeUiResponse {
        let dot_id = uictx.egui_id().with("dot");
        let framed = uictx.framed(|ui, _sockets| {
            kit::event_label(ui, dot_id, Some("tick"), egui::Sense::hover())
        });
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
                    .with_description("tick duration in seconds, emitted once per elapsed tick"),
            ),
            gantz_egui::SocketKind::Input => None,
        }
    }
}

/// Drives `tick!` nodes every update, independent of GUI visibility.
///
/// Advances each `tick!` node's timer by the update delta time and triggers
/// one push evaluation for every whole tick duration elapsed. Catch-up is
/// capped by `MAX_CATCHUP_TICKS`. Any backlog beyond the cap is dropped.
pub fn drive_tick_bangs(
    time: Res<Time>,
    epoch: Res<bevy_gantz::EvalEpoch>,
    mut ticks: Query<(Entity, Ref<TickBang>, &mut TickTimer)>,
    mut cmds: Commands,
) {
    // The frame's monotonic now. Each tick's exact firing time derives from it.
    let now = epoch.now_secs();
    for (node, tick, mut timer) in ticks.iter_mut() {
        // The inspector clamps to `MIN_DURATION`, but stored data may hold
        // any duration.
        let dur = tick.duration();
        let Some(duration) = Duration::try_from_secs_f64(dur)
            .ok()
            .filter(|d| !d.is_zero())
        else {
            continue;
        };
        if tick.is_changed() {
            timer.0.set_duration(duration);
        }
        timer.0.tick(time.delta());
        let n = timer.0.times_finished_this_tick().min(MAX_CATCHUP_TICKS);
        // The time since the most recent tick boundary.
        let since = timer.0.elapsed_secs_f64();
        // Trigger one eval per elapsed tick, oldest first. Each is stamped
        // with the exact monotonic time it should fire. The i-th most recent
        // tick fired at `now - since - i*dur`, with i = 0 the latest.
        // Per-tick times let the dsp driver schedule them sample-accurately
        // across the interval rather than bunched at the frame boundary.
        for i in (0..n).rev() {
            let time = Some(now - since - f64::from(i) * dur);
            cmds.trigger(PushEval { entity: node, time });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::{App, Update};
    use bevy_time::TimeUpdateStrategy;

    /// The times of the push evaluations of the update.
    #[derive(Default, Resource)]
    struct Pushes(Vec<f64>);

    /// An app that drives one `tick!` node of `secs` per tick, and records
    /// its pushes.
    fn app(secs: f64) -> App {
        let mut app = App::new();
        app.add_plugins(bevy_time::TimePlugin)
            .insert_resource(bevy_gantz::EvalEpoch(web_time::Instant::now()))
            .init_resource::<Pushes>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
            .add_systems(Update, drive_tick_bangs)
            .add_observer(|on: On<PushEval>, mut pushes: ResMut<Pushes>| {
                pushes.0.extend(on.time);
            });
        let mut tick = TickBang::default();
        tick.set_interval(Interval::Duration(secs));
        app.world_mut().spawn(tick);
        app.update();
        app
    }

    /// The pushes of one update that advances the clock by `secs`.
    fn advance(app: &mut App, secs: f64) -> Vec<f64> {
        let step = Duration::from_secs_f64(secs);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(step));
        app.update();
        std::mem::take(&mut app.world_mut().resource_mut::<Pushes>().0)
    }

    /// An update fires once per whole tick that it passes, each at the time
    /// of its own tick, oldest first.
    #[test]
    fn ticks_fire_once_each_at_their_times() {
        let mut app = app(0.05);
        let times = advance(&mut app, 0.175);
        assert_eq!(times.len(), 3, "{times:?}");
        let gaps: Vec<f64> = times.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|g| (g - 0.05).abs() < 1e-9), "{gaps:?}");
        assert_eq!(advance(&mut app, 0.03).len(), 1, "the remainder carries");
    }

    /// An update that passes more ticks than `MAX_CATCHUP_TICKS` fires that
    /// many, and drops the rest. Bevy clamps the delta of an update to a
    /// quarter second, so the ticks are short.
    #[test]
    fn a_stall_catches_up_to_the_cap() {
        let mut app = app(MIN_DURATION);
        assert_eq!(advance(&mut app, 0.2).len(), MAX_CATCHUP_TICKS as usize);
        assert!(advance(&mut app, 0.0).is_empty());
    }
}
