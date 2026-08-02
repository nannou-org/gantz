//! The `sleep` node: produce a gantz task that resolves to the input value
//! after a configurable duration.
//!
//! The simplest task producer, useful for testing and demonstrating `await`.
//! The task is a driver-polled deadline check (see
//! [`GantzTask::poll_fn`](bevy_gantz::task::GantzTask::poll_fn)) rather than
//! an executor-driven future, so it is portable to single-threaded targets
//! and costs nothing while pending.

use bevy_egui::egui;
use bevy_gantz::task::{GantzTask, TaskHandle};
use gantz_core::node::{self, EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_format::{Datum, FormatError, SugarArgs, node_datum};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use steel::SteelVal;
use steel::steel_vm::register_fn::RegisterFn;

/// The default sleep duration in seconds when unconfigured.
const DEFAULT_DURATION: f64 = 1.0;

/// The smallest sleep duration the inspector allows, in seconds.
const MIN_DURATION: f64 = 0.0;

/// The name of the registered fn producing the sleep task.
const SLEEP_FN: &str = "gantz-sleep";

// ---------------------------------------------------------------------------
// Sleep node
// ---------------------------------------------------------------------------

/// A node producing a gantz task that resolves to the received input value
/// after the configured duration.
///
/// Push a value into it (e.g. from a `bang`) and wire the task output into an
/// `await` node to receive the value once the duration elapses.
#[derive(Clone, Debug, Serialize, Deserialize, NodeTag)]
pub struct Sleep {
    #[serde(
        default = "default_duration",
        skip_serializing_if = "is_default_duration"
    )]
    duration: f64,
}

fn default_duration() -> f64 {
    DEFAULT_DURATION
}

fn is_default_duration(duration: &f64) -> bool {
    *duration == DEFAULT_DURATION
}

impl Sleep {
    /// The sleep duration in seconds.
    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// Set the sleep duration in seconds (content-address affecting).
    pub fn set_duration(&mut self, duration: f64) {
        self.duration = duration.max(MIN_DURATION);
    }
}

impl Default for Sleep {
    fn default() -> Self {
        Sleep {
            duration: DEFAULT_DURATION,
        }
    }
}

impl PartialEq for Sleep {
    fn eq(&self, other: &Self) -> bool {
        self.duration.to_bits() == other.duration.to_bits()
    }
}

impl Eq for Sleep {}

impl Hash for Sleep {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Hash::hash(&self.duration.to_bits(), state);
    }
}

impl gantz_core::Node for Sleep {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        vec![EvalConf::All]
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        // The forwarded value is the input when connected, else `'()`.
        // `{:?}` formats the float with a guaranteed `.`/exponent so Steel
        // parses it as a number rather than an integer.
        let val = match ctx.inputs().first() {
            Some(Some(input)) => input.as_str(),
            _ => "'()",
        };
        node::parse_expr(&format!("({SLEEP_FN} {:?} {val})", self.duration))
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let vm = ctx.vm();
        bevy_gantz::task::register_task_type(vm);
        // Register the producer fn only once: `register_fn` allocates a new
        // global slot and shadows the previous binding rather than
        // overwriting it, so re-running this on every recompile would leak.
        if vm.extract_value(SLEEP_FN).is_err() {
            vm.register_fn(SLEEP_FN, gantz_sleep);
        }
    }
}

impl gantz_egui::NodeUi for Sleep {
    fn name(&self, _: &gantz_egui::Env<'_>) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("sleep")
    }

    fn description(&self) -> Option<&'static str> {
        Some(
            "Produces a gantz task that resolves to the received input value \
             after the configured duration. Wire the output into an `await` \
             node to receive the value once the duration elapses.",
        )
    }

    fn ui(
        &mut self,
        _ctx: gantz_egui::NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> gantz_egui::NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("sleep").selectable(false)));
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
                ui.label("dur.")
                    .on_hover_text("seconds until the task resolves");
            });
            row.col(|ui| {
                let mut v = self.duration;
                let resp = ui.add(
                    egui::DragValue::new(&mut v)
                        .speed(0.01)
                        .range(MIN_DURATION..=f64::INFINITY)
                        .suffix(" s"),
                );
                if resp.changed() {
                    self.set_duration(v);
                    changed = true;
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
            gantz_egui::SocketKind::Input => Some(
                gantz_egui::SocketDoc::ty("any")
                    .with_description("value the task resolves to after the duration"),
            ),
            gantz_egui::SocketKind::Output => Some(
                gantz_egui::SocketDoc::ty("task")
                    .with_description("gantz task resolving to the input after the duration"),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// `.gantz` keyword sugar
// ---------------------------------------------------------------------------

/// Read a `(sleep [#:duration secs])` form into a [`Sleep`] datum. No
/// `#:duration` yields the default. Dispatched by [`crate::sugar::BevySugar`].
pub(crate) fn read_sugar(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let fields = match args.keyword_f64("duration")? {
        Some(secs) => vec![("duration", Datum::F64(secs))],
        None => vec![],
    };
    Ok(node_datum("Sleep", fields))
}

/// Write a [`Sleep`] as bare `sleep` for the default duration, else as
/// `(sleep #:duration secs)`.
pub(crate) fn write_sugar(node: &Datum) -> String {
    match node.get("duration").and_then(Datum::as_f64) {
        Some(secs) if secs != DEFAULT_DURATION => format!("(sleep #:duration {secs})"),
        _ => "sleep".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Registered fns
// ---------------------------------------------------------------------------

/// Produce a task resolving to `val` once `secs` seconds have passed.
///
/// Non-finite or negative durations resolve immediately.
fn gantz_sleep(secs: f64, val: SteelVal) -> TaskHandle {
    let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    let deadline = web_time::Instant::now() + std::time::Duration::from_secs_f64(secs);
    let mut val = Some(val);
    TaskHandle::new(GantzTask::poll_fn(move || {
        if web_time::Instant::now() < deadline {
            return None;
        }
        val.take().map(Ok)
    }))
}
