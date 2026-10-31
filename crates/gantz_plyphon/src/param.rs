//! Helpers for DSP node parameters. They build a plyphon control param, name
//! it uniquely within a synthdef, fold its smoothing lag into a content
//! address, and read and write the param's node state.
//!
//! A DSP param's value lives in the node's state, path-keyed like `number`,
//! so editing it does not churn the graph's content address. Its lag lives
//! in the node's fields, because lag is structural. It bakes a `LagControl`
//! into the synthdef and so respawns on change.
//!
//! Two state shapes exist.
//! - Bare. The node's whole state is one `{ value, pending }` map, the
//!   single-param shape. `~out`'s gain uses it.
//! - Keyed. The node's state maps param names to `{ value, pending }`
//!   sub-maps, for example `{ 'freq { .. }, 'width { .. } }`, for nodes with
//!   several params. [`params_state`] and [`control_inputs_expr`] operate on
//!   this shape. A [`ParamBinding`](crate::ParamBinding) records which shape
//!   feeds each synth param via its `key`.

use gantz_core::datum::{self, Datum};
use gantz_core::steel::steel_vm::engine::Engine;
use gantz_nodedef::ExprArgs;
use plyphon::synthdef::Param;

/// The `value` key of a DSP param's state, the current scalar value. The
/// inspector reads and writes it. The runtime uses it as the immediate
/// fallback.
pub(crate) const VALUE: &str = "value";
/// The `pending` key, a list of `(time value)` updates a connected control
/// input has queued since the last frame. The runtime drains and schedules
/// them.
const PENDING: &str = "pending";

/// The plyphon control [`Param`] named `name` with the given default value and
/// one-pole smoothing `lag` in seconds. A `lag` of `0.0` is a plain control.
pub fn plyphon_param(name: impl Into<String>, default: f32, lag: f32) -> Param {
    if lag > 0.0 {
        Param::lag(name, default, lag)
    } else {
        Param::control(name, default)
    }
}

/// Build a single-param DSP node's Steel `expr`.
///
/// When the node's control input at `control_ix` is connected, a numeric
/// incoming value is queued onto the param state's `pending` list. The entry
/// is tagged with this evaluation's firing time from `(hash-ref %args 'time)`.
/// The value is also written to the param's current `value`. The audio driver
/// drains `pending` each frame and schedules each `(time value)` ahead of the
/// audio clock. A `tick!`-driven chain therefore animates the param
/// sample-accurately rather than bunched at the frame boundary. `value` is
/// the driver's immediate fallback for direct inspector edits. The expr
/// always evaluates to `output`, the node's placeholder dsp output. That is
/// `"state"` for a source like `~sinosc` and `"'()"` for a sink like `~out`.
/// DSP nodes are otherwise Steel-inert.
///
/// A non-empty list of 2-element numeric `(time value)` lists, the shape
/// `pat/events->secs` emits, is queued as a pre-timestamped batch instead.
/// `value` takes the batch's last value. A whole pattern window then lands
/// sample-accurately from a single eval.
///
/// The `number?` guard is what makes a hybrid input work, see
/// [`NodeDsp::n_dsp_inputs`](crate::NodeDsp::n_dsp_inputs). A connected dsp
/// source's placeholder output is non-numeric by contract and is ignored
/// here. So is the list of bare values a multi-edge input evaluates to, since
/// its head is not a `(time value)` pair. While any dsp wire shares the input
/// the wire wins. Numbers are not queued, matching the derived def, where the
/// param is absent and nothing would drain the queue.
pub fn control_input_expr(ctx: &ExprArgs<'_>, control_ix: usize, output: &str) -> String {
    match ctx.inputs.get(control_ix) {
        Some(Some(val)) => {
            let time = format!("(hash-ref {} '{})", ctx.args, gantz_core::args::TIME);
            let single = format!(
                "(set! state \
                   (hash-insert \
                     (hash-insert state '{VALUE} {val}) \
                     '{PENDING} \
                     (cons (list {time} {val}) (hash-ref state '{PENDING}))))"
            );
            let batch = format!(
                "(let ((rev (reverse {val}))) \
                   (set! state \
                     (hash-insert \
                       (hash-insert state '{VALUE} (car (cdr (car rev)))) \
                       '{PENDING} \
                       (append rev (hash-ref state '{PENDING})))))"
            );
            let queue = guarded_queue_expr(val, &single, &batch);
            format!("(begin {queue} {output})")
        }
        _ => format!("(begin {output})"),
    }
}

/// The queueing dispatch for a connected control input's value `val`.
///
/// A number runs `single`, the one-pair eval-timestamped queue write. A
/// non-empty list whose head is a 2-element numeric `(time value)` list
/// runs `batch`, the pre-timestamped shape `pat/events->secs` emits.
/// Anything else, such as a dsp placeholder or a multi-edge list of bare
/// values, is ignored.
fn guarded_queue_expr(val: &str, single: &str, batch: &str) -> String {
    format!(
        "(if (number? {val}) \
             {single} \
             (if (list? {val}) \
                 (if (empty? {val}) \
                     void \
                     (if (list? (car {val})) \
                         (if (number? (car (car {val}))) \
                             (if (number? (car (cdr (car {val})))) \
                                 {batch} \
                                 void) \
                             void) \
                         void)) \
                 void))"
    )
}

/// Build a multi-param DSP node's Steel `expr` over keyed state.
///
/// The keyed analogue of [`control_input_expr`]. `inputs` pairs each hybrid
/// control input's index with its param name. Each connected input queues and
/// writes into the `{ value, pending }` sub-map at its name within the node's
/// keyed state, see the module docs. Unconnected inputs emit nothing. The
/// same `number?` guard and timestamped `pending` queue semantics apply per
/// input. The expr always evaluates to `output`.
pub fn control_inputs_expr(ctx: &ExprArgs<'_>, inputs: &[(usize, &str)], output: &str) -> String {
    let time = format!("(hash-ref {} '{})", ctx.args, gantz_core::args::TIME);
    let mut expr = "(begin ".to_string();
    for (control_ix, name) in inputs {
        let Some(Some(val)) = ctx.inputs.get(*control_ix) else {
            continue;
        };
        let sub = format!("(hash-ref state '{name})");
        let single = format!(
            "(set! state \
               (hash-insert state '{name} \
                 (hash-insert \
                   (hash-insert {sub} '{VALUE} {val}) \
                   '{PENDING} \
                   (cons (list {time} {val}) (hash-ref {sub} '{PENDING})))))"
        );
        let batch = format!(
            "(let ((rev (reverse {val}))) \
               (set! state \
                 (hash-insert state '{name} \
                   (hash-insert \
                     (hash-insert {sub} '{VALUE} (car (cdr (car rev)))) \
                     '{PENDING} \
                     (append rev (hash-ref {sub} '{PENDING}))))))"
        );
        expr.push_str(&guarded_queue_expr(val, &single, &batch));
        expr.push(' ');
    }
    expr.push_str(output);
    expr.push(')');
    expr
}

/// A synthdef parameter name unique to a node's parameter within a synthdef,
/// for example `"2/freq"` for the `freq` param of the node at path `[2]`.
pub fn param_name(path: &[usize], param: &str) -> String {
    let prefix = path
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("-");
    format!("{prefix}/{param}")
}

/// The state of a DSP param, a map `{ value, pending }`. `value` is the
/// current scalar, seeded with `default`. `pending` starts as an empty list
/// of the `(time value)` updates that a control input queues.
pub fn param_state(default: f64) -> Datum {
    map(vec![
        (VALUE.to_string(), Datum::F64(default)),
        (PENDING.to_string(), Datum::Seq(vec![])),
    ])
}

/// The keyed state of a multi-param DSP node, a map from param name to a
/// [`param_state`] sub-map, one entry per `(name, default)`.
pub fn params_state(defaults: &[(&str, f64)]) -> Datum {
    let entries = defaults
        .iter()
        .map(|(name, default)| (name.to_string(), param_state(*default)));
    map(entries.collect())
}

/// The keyed state of a node whose param set follows its fields, given its
/// state so far. Each `keep` param keeps its sub-map in `prev`, or starts at
/// its default. Each `set` param takes the given value and keeps its queue.
/// Any other param in `prev` is dropped.
pub fn sync_params_state(
    prev: Option<&Datum>,
    keep: &[(&str, f64)],
    set: &[(String, f64)],
) -> Datum {
    let old = |name: &str| prev.and_then(|prev| prev.get(name));
    let mut entries: Vec<(String, Datum)> = vec![];
    for (name, default) in keep {
        let sub = old(name).cloned().unwrap_or_else(|| param_state(*default));
        insert(&mut entries, name, sub);
    }
    for (name, value) in set {
        let sub = match old(name) {
            Some(sub) => with_value(sub, *value),
            None => param_state(*value),
        };
        insert(&mut entries, name, sub);
    }
    map(entries)
}

/// The param state `state` with its value set to `value`. Its queue stays. A
/// state that is not a map becomes a fresh one.
pub fn with_value(state: &Datum, value: f64) -> Datum {
    let Datum::Map(entries) = state else {
        return param_state(value);
    };
    let mut entries = entries.clone();
    insert(&mut entries, VALUE, Datum::F64(value));
    map(entries)
}

/// Set `key` to `value` in the entries of a map, in place of any entry with
/// the key.
fn insert(entries: &mut Vec<(String, Datum)>, key: &str, value: Datum) {
    match entries.iter_mut().find(|(k, _)| k == key) {
        Some((_, v)) => *v = value,
        None => entries.push((key.to_string(), value)),
    }
}

/// A DSP param's current value and its queued `(time, value)` updates,
/// oldest first, read from the node state `state`. `key` names the param
/// within keyed state, see the module docs. When any updates were queued,
/// also the node state with the param's queue cleared, for the caller to
/// write back. A bare number reads as the value with no queue. `None` when
/// the state holds no such param.
pub fn drain(state: &Datum, key: Option<&str>) -> Option<(f64, Vec<(f64, f64)>, Option<Datum>)> {
    let Some(key) = key else {
        return drain_sub(state);
    };
    let Datum::Map(entries) = state else {
        return None;
    };
    let ix = entries.iter().position(|(k, _)| k == key)?;
    let (value, pending, cleared) = drain_sub(&entries[ix].1)?;
    let cleared = cleared.map(|sub| {
        let mut entries = entries.clone();
        entries[ix].1 = sub;
        map(entries)
    });
    Some((value, pending, cleared))
}

/// Read one `{ value, pending }` sub-map. See [`drain`].
fn drain_sub(sub: &Datum) -> Option<(f64, Vec<(f64, f64)>, Option<Datum>)> {
    let Datum::Map(entries) = sub else {
        return sub.as_f64().map(|v| (v, Vec::new(), None));
    };
    let value = sub.get(VALUE)?.as_f64()?;
    let mut pending: Vec<(f64, f64)> = match sub.get(PENDING) {
        Some(Datum::Seq(list)) => list
            .iter()
            .filter_map(|elem| {
                let Datum::Seq(pair) = elem else {
                    return None;
                };
                let t = pair.first()?.as_f64()?;
                let v = pair.get(1)?.as_f64()?;
                Some((t, v))
            })
            .collect(),
        _ => Vec::new(),
    };
    // The queue is built by prepending, latest first. Return oldest-first.
    pending.reverse();
    let cleared = (!pending.is_empty()).then(|| {
        let entries = entries.iter().map(|(k, v)| match k == PENDING {
            true => (k.clone(), Datum::Seq(vec![])),
            false => (k.clone(), v.clone()),
        });
        map(entries.collect())
    });
    Some((value, pending, cleared))
}

/// A DSP param's value and its queued updates, oldest first, drained from
/// the state of the node at `path` in `vm`, which takes the cleared state
/// back. See [`drain`]. `None` when the node has no such param.
pub fn drain_param(vm: &mut Engine, path: &[usize]) -> Option<(f64, Vec<(f64, f64)>)> {
    drain_at(vm, path, None)
}

/// The keyed analogue of [`drain_param`], for the param `name`.
pub fn drain_param_keyed(
    vm: &mut Engine,
    path: &[usize],
    name: &str,
) -> Option<(f64, Vec<(f64, f64)>)> {
    drain_at(vm, path, Some(name))
}

/// The number of queued updates in a param state, without draining them. `0`
/// for a bare number, a missing queue or an empty one.
pub fn pending_len(state: &Datum) -> usize {
    match state.get(PENDING) {
        Some(Datum::Seq(list)) => list.len(),
        _ => 0,
    }
}

/// The queued updates across every param of a keyed state, without draining
/// them.
pub fn pending_len_total(state: &Datum) -> usize {
    match state {
        Datum::Map(entries) => entries.iter().map(|(_, v)| pending_len(v)).sum(),
        _ => 0,
    }
}

/// See [`drain_param`].
fn drain_at(vm: &mut Engine, path: &[usize], key: Option<&str>) -> Option<(f64, Vec<(f64, f64)>)> {
    let state = gantz_core::node::state::extract_value(vm, path)
        .ok()
        .flatten()?;
    let state = datum::of_steel(&state).ok()?;
    let (value, pending, cleared) = drain(&state, key)?;
    if let Some(cleared) = cleared {
        let _ = gantz_core::node::state::update_value(vm, path, datum::to_steel(&cleared));
    }
    Some((value, pending))
}

/// The map datum of `entries`, sorted by key, as a state that is read from
/// Steel is.
pub(crate) fn map(mut entries: Vec<(String, Datum)>) -> Datum {
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));
    Datum::Map(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::steel::SteelVal;
    use gantz_core::steel::steel_vm::engine::Engine;

    /// Run the expr that `expr` builds for a node at `[0]` with the state
    /// `state` and its input 0 bound to the Steel expression `input`, as the
    /// codegen's stateful wrapper does. The eval time is 7.0. Returns the
    /// state after.
    fn run(state: Datum, expr: impl Fn(&ExprArgs<'_>) -> String, input: &str) -> Datum {
        let mut vm = Engine::new_base();
        vm.register_value(gantz_core::ROOT_STATE, SteelVal::empty_hashmap());
        vm.register_value(gantz_core::ARGS, gantz_core::args::time(7.0));
        let state = datum::to_steel(&state);
        gantz_core::node::state::update_value(&mut vm, &[0], state).unwrap();
        let inputs = [Some(input)];
        let ctx = ExprArgs {
            inputs: &inputs,
            outputs: &[true],
            path: &[0],
            args: gantz_core::ARGS,
        };
        let src = format!(
            "(define state (hash-ref {root} 0))
             {}
             (set! {root} (hash-insert {root} 0 state))",
            expr(&ctx),
            root = gantz_core::ROOT_STATE,
        );
        vm.run(src).expect("run the expr");
        crate::state_value(&vm, &[0]).expect("the state")
    }

    /// A param's value and queue, oldest first.
    fn read(state: &Datum, key: Option<&str>) -> Option<(f64, Vec<(f64, f64)>)> {
        drain(state, key).map(|(value, pending, _)| (value, pending))
    }

    /// A connected number queues one pair stamped with the eval time. A batch
    /// of `(time value)` pairs, the `pat/events->secs` shape, is queued whole,
    /// draining oldest-first, with `value` taking the last. Multi-edge lists
    /// of bare values, empty lists and non-numeric values are all ignored.
    /// Nothing is queued and the value keeps its initial 1.0.
    #[test]
    fn control_input_queues_numbers_and_batches_only() {
        let cases: &[(&str, f64, &[(f64, f64)])] = &[
            ("42.0", 42.0, &[(7.0, 42.0)]),
            (
                "(list (list 1.0 10.0) (list 2.0 20.0) (list 3.0 30.0))",
                30.0,
                &[(1.0, 10.0), (2.0, 20.0), (3.0, 30.0)],
            ),
            ("(list 1.0 2.0)", 1.0, &[]),
            ("(list)", 1.0, &[]),
            ("'sym", 1.0, &[]),
            ("(list (list 'x 1.0))", 1.0, &[]),
        ];
        for &(input, value, pending) in cases {
            let expr = |ctx: &ExprArgs<'_>| control_input_expr(ctx, 0, "state");
            let state = run(param_state(1.0), expr, input);
            assert_eq!(
                read(&state, None),
                Some((value, pending.to_vec())),
                "{input}"
            );
        }
    }

    /// The keyed emitter queues a batch into the named param's sub-map
    /// only, leaving siblings untouched.
    #[test]
    fn control_inputs_keyed_batch() {
        let state = params_state(&[("freq", 220.0), ("width", 0.5)]);
        let expr = |ctx: &ExprArgs<'_>| control_inputs_expr(ctx, &[(0, "freq")], "state");
        let state = run(state, expr, "(list (list 1.0 330.0) (list 2.0 440.0))");
        let freq = (440.0, vec![(1.0, 330.0), (2.0, 440.0)]);
        assert_eq!(read(&state, Some("freq")), Some(freq));
        assert_eq!(read(&state, Some("width")), Some((0.5, vec![])));
    }

    /// A `{ value, pending }` sub-map with the given queue, stored
    /// newest-first as the expr prepends.
    fn sub(value: f64, newest_first: &[(f64, f64)]) -> Datum {
        let pair = |&(t, v): &(f64, f64)| Datum::Seq(vec![Datum::F64(t), Datum::F64(v)]);
        map(vec![
            (VALUE.to_string(), Datum::F64(value)),
            (
                PENDING.to_string(),
                Datum::Seq(newest_first.iter().map(pair).collect()),
            ),
        ])
    }

    /// A kept param keeps its sub-map, a new one starts at its default, a set
    /// one takes the given value and keeps its queue, and a stale one goes.
    #[test]
    fn sync_params_state_follows_the_node_data() {
        let prev = map(vec![
            ("gate".into(), sub(0.0, &[(1.0, 1.0)])),
            ("level-0".into(), sub(0.5, &[(2.0, 0.5)])),
            ("level-1".into(), sub(0.2, &[])),
        ]);
        let set = vec![("level-0".to_string(), 0.9)];
        let state = sync_params_state(Some(&prev), &[("gate", 1.0), ("bias", 0.0)], &set);
        assert_eq!(read(&state, Some("gate")), Some((0.0, vec![(1.0, 1.0)])));
        assert_eq!(read(&state, Some("bias")), Some((0.0, vec![])));
        assert_eq!(read(&state, Some("level-0")), Some((0.9, vec![(2.0, 0.5)])));
        assert_eq!(read(&state, Some("level-1")), None);
        let fresh = sync_params_state(None, &[("gate", 1.0)], &set);
        assert_eq!(read(&fresh, Some("gate")), Some((1.0, vec![])));
        assert_eq!(read(&fresh, Some("level-0")), Some((0.9, vec![])));
    }

    #[test]
    fn drain_drains_only_the_named_queue() {
        let state = map(vec![
            ("freq".into(), sub(220.0, &[(1.0, 330.0), (0.5, 300.0)])),
            ("width".into(), sub(0.5, &[(0.7, 0.25)])),
        ]);

        // Draining `freq` yields its value and queue oldest-first.
        let (value, pending, cleared) = drain(&state, Some("freq")).unwrap();
        assert_eq!(value, 220.0);
        assert_eq!(pending, vec![(0.5, 300.0), (1.0, 330.0)]);

        // It clears only `freq`'s queue.
        let state = cleared.expect("a queue was drained");
        assert_eq!(drain(&state, Some("freq")), Some((220.0, vec![], None)));
        assert_eq!(read(&state, Some("width")), Some((0.5, vec![(0.7, 0.25)])));
        assert_eq!(drain(&state, Some("nope")), None);
    }

    /// Bare state drains as one param, and a bare number is its value.
    #[test]
    fn drain_reads_bare_state() {
        let (value, pending, cleared) = drain(&sub(1.0, &[(2.0, 3.0)]), None).unwrap();
        assert_eq!((value, pending), (1.0, vec![(2.0, 3.0)]));
        assert_eq!(cleared, Some(sub(1.0, &[])));
        assert_eq!(drain(&Datum::I64(4), None), Some((4.0, vec![], None)));
        assert_eq!(drain(&Datum::Str("x".into()), None), None);
        assert_eq!(
            with_value(&sub(1.0, &[(2.0, 3.0)]), 5.0),
            sub(5.0, &[(2.0, 3.0)])
        );
    }
}
