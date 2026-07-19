//! Reusable helpers for DSP node parameters: building a plyphon control param,
//! naming it uniquely within a synthdef and folding its smoothing lag into a
//! content address. The inspector rows over this state live in `crate::egui`
//! (`egui` feature).
//!
//! A DSP param's *value* lives in the node's VM state (path-keyed, like `number`),
//! so editing it does not churn the graph's content address. Its *lag* lives in
//! the node weight, because lag is structural (it bakes a `LagControl` into the
//! synthdef and so respawns on change).

use gantz_core::node::{ExprCtx, ExprResult};
use gantz_core::steel::gc::Gc;
use gantz_core::steel::steel_vm::engine::Engine;
use gantz_core::steel::{HashMap, SteelVal};
use plyphon::synthdef::Param;

/// The `value` key of a DSP param's structured VM state: the current scalar value
/// (the inspector reads/writes it. The driver uses it as the immediate fallback).
const VALUE: &str = "value";
/// The `pending` key: a list of `(time value)` updates a connected control input
/// has queued since the last frame, drained and scheduled by the audio driver.
const PENDING: &str = "pending";

/// The plyphon control [`Param`] named `name` with the given default value and
/// optional one-pole smoothing `lag` (seconds, `0.0` = a plain control).
pub fn plyphon_param(name: impl Into<String>, default: f32, lag: f32) -> Param {
    if lag > 0.0 {
        Param::lag(name, default, lag)
    } else {
        Param::control(name, default)
    }
}

/// Build a single-param DSP node's Steel `expr`.
///
/// If the node's *control input* at `control_ix` is connected, the incoming value
/// is, guarded by `number?`: (1) queued onto the param state's `pending` list,
/// tagged with this evaluation's firing time (`(hash-ref %args 'time)`), and (2)
/// written to the param's current `value`. The audio driver drains `pending` each
/// frame and *schedules* each `(time value)` ahead of the audio clock, so a
/// `tick!`-driven chain animates the param sample-accurately rather than bunched at
/// the frame boundary. `value` is the driver's immediate fallback for direct
/// inspector edits. The expr always evaluates to `output`: the node's placeholder
/// dsp output (`"state"` for a source like `~sinosc`, `"'()"` for a sink like
/// `~out`). DSP nodes are otherwise Steel-inert.
///
/// The `number?` guard is what makes a *hybrid* input work (a dsp input that
/// falls back to a control param, e.g. `~sinosc`'s freq - see
/// [`NodeDsp::n_dsp_inputs`](crate::NodeDsp::n_dsp_inputs)): a connected dsp
/// source's placeholder output is non-numeric by contract and is ignored here,
/// as is the list a multi-edge input evaluates to. So while any dsp wire shares
/// the input the wire wins - numbers are not queued, matching the derived def,
/// where the param is absent and nothing would drain the queue.
pub fn control_input_expr(ctx: &ExprCtx<'_, '_>, control_ix: usize, output: &str) -> ExprResult {
    let expr = match ctx.inputs().get(control_ix) {
        Some(Some(val)) => {
            let time = format!("(hash-ref {} '{})", ctx.args(), gantz_core::args::TIME);
            format!(
                "(begin \
                   (if (number? {val}) \
                       (set! state \
                         (hash-insert \
                           (hash-insert state '{VALUE} {val}) \
                           '{PENDING} \
                           (cons (list {time} {val}) (hash-ref state '{PENDING})))) \
                       void) \
                   {output})"
            )
        }
        _ => format!("(begin {output})"),
    };
    gantz_core::node::parse_expr(&expr)
}

/// A synthdef parameter name unique to a node's parameter within a synthdef,
/// e.g. `"2/freq"` for the `freq` param of the node at path `[2]`.
pub fn param_name(path: &[usize], param: &str) -> String {
    let prefix = path
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("-");
    format!("{prefix}/{param}")
}

/// The structured VM state of a DSP param: a hashmap `{ value, pending }`.
///
/// `value` is the current scalar (seeded with `default`). `pending` starts as an
/// empty list of `(time value)` updates a control input will queue. Seed it from a
/// node's `register` (mirrors `~sinosc`).
pub fn param_state(default: f64) -> SteelVal {
    let map = HashMap::new()
        .update(sym(VALUE), SteelVal::NumV(default))
        .update(sym(PENDING), empty_list());
    SteelVal::HashMapV(Gc::new(map).into())
}

/// Read a DSP param's current `value` from its structured state, if present.
pub fn param_value(state: &SteelVal) -> Option<f64> {
    match state {
        SteelVal::HashMapV(map) => map.get(&sym(VALUE)).and_then(steel_num),
        // Tolerate a bare scalar (e.g. older state) as the value.
        other => steel_num(other),
    }
}

/// The number of queued `pending` control updates in a DSP param's state, WITHOUT
/// draining them (a non-mutating peek for a UI readout). `0` for a bare scalar, a
/// missing queue, or an empty one.
pub fn pending_len(state: &SteelVal) -> usize {
    match state {
        SteelVal::HashMapV(map) => match map.get(&sym(PENDING)) {
            Some(SteelVal::ListV(list)) => list.len(),
            _ => 0,
        },
        _ => 0,
    }
}

/// Set a DSP param's `value`, preserving its `pending` queue. Used by the inspector
/// on a direct edit (the value is not content-addressed).
pub fn with_value(state: SteelVal, value: f64) -> SteelVal {
    let map = match state {
        SteelVal::HashMapV(map) => map.update(sym(VALUE), SteelVal::NumV(value)),
        _ => HashMap::new()
            .update(sym(VALUE), SteelVal::NumV(value))
            .update(sym(PENDING), empty_list()),
    };
    SteelVal::HashMapV(Gc::new(map).into())
}

/// Read a DSP param's `value` and drain its `pending` queue from VM state, clearing
/// `pending` (writing the cleared state back). Returns `None` if the node has no
/// state. Drained updates are returned oldest-first.
pub fn drain_param(vm: &mut Engine, path: &[usize]) -> Option<(f64, Vec<(f64, f64)>)> {
    let state = gantz_core::node::state::extract_value(vm, path)
        .ok()
        .flatten()?;
    let SteelVal::HashMapV(map) = &state else {
        // A bare scalar still yields a value (no queue).
        return steel_num(&state).map(|v| (v, Vec::new()));
    };
    let value = map.get(&sym(VALUE)).and_then(steel_num)?;
    let mut pending: Vec<(f64, f64)> = match map.get(&sym(PENDING)) {
        Some(SteelVal::ListV(list)) => list
            .iter()
            .filter_map(|elem| {
                let SteelVal::ListV(pair) = elem else {
                    return None;
                };
                let mut it = pair.iter();
                let t = it.next().and_then(steel_num)?;
                let v = it.next().and_then(steel_num)?;
                Some((t, v))
            })
            .collect(),
        _ => Vec::new(),
    };
    // The queue is built by prepending (latest first). Return oldest-first.
    pending.reverse();
    if !pending.is_empty() {
        let cleared = map.update(sym(PENDING), empty_list());
        let _ = gantz_core::node::state::update_value(
            vm,
            path,
            SteelVal::HashMapV(Gc::new(cleared).into()),
        );
    }
    Some((value, pending))
}

/// A symbol [`SteelVal`] for a state key (matching the Steel `'value`/`'pending`).
fn sym(name: &str) -> SteelVal {
    SteelVal::SymbolV(name.into())
}

/// An empty Steel list - the initial `pending` queue.
fn empty_list() -> SteelVal {
    SteelVal::ListV(Default::default())
}

/// Convert a numeric [`SteelVal`] to `f64` (handles `NumV` and `IntV`).
fn steel_num(val: &SteelVal) -> Option<f64> {
    match val {
        SteelVal::NumV(f) => Some(*f),
        SteelVal::IntV(i) => Some(*i as f64),
        _ => None,
    }
}
