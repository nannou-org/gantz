//! The `~buffer` scratch buffer source node.
//!
//! A `~buffer` can also hold a table. A list of numbers on its control input
//! is kept in the node's VM state as `{ table, dirty }`. The audio driver
//! writes the table into the buffer when `dirty` is set, and into each new
//! buffer of the node. See [`take_dirty`], [`table`] and [`table_samples`].

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::steel::gc::Gc;
use gantz_core::steel::steel_vm::engine::Engine;
use gantz_core::steel::{HashMap, SteelVal};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

use crate::dsp::{BufferSource, DspBuilder, NodeDsp, Signal, ToNodeDsp};
use crate::param::{steel_num, sym};

/// A zeroed scratch buffer as a buffer source. It emits the bufnum wire of
/// the buffer, for example for a `~recordbuf` that writes it and a
/// `~playbuf` that reads it.
///
/// The audio driver allocates one buffer per open head and node path, so
/// each instance of a nested graph gets its own buffer. The buffer keeps its
/// contents when the synths that use it respawn. A change of `frames`,
/// `channels` or `wavetable` gives a new buffer.
///
/// A list of numbers on the control input is a table. The driver writes it
/// into the buffer as interleaved samples and sets the rest to zero. With
/// `wavetable` set, the driver first converts the list to the format that
/// `~osc` and `~cosc` read. This doubles its length. These units also need
/// `frames` to be a power of two.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, NodeTag)]
pub struct Buffer {
    #[serde(default = "default_frames", skip_serializing_if = "is_default_frames")]
    frames: usize,
    #[serde(
        default = "default_channels",
        skip_serializing_if = "is_default_channels"
    )]
    channels: usize,
    #[serde(default, skip_serializing_if = "is_false")]
    wavetable: bool,
}

/// A table element that is not a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("table element {index} is not a number")]
pub struct TableError {
    /// The index of the element in the list.
    pub index: usize,
}

/// The state key of the latest table, a list of numbers.
const TABLE: &str = "table";

/// The state key that is true when a table arrived after the driver last
/// took it.
const DIRTY: &str = "dirty";

impl Buffer {
    /// The frame count a fresh `~buffer` starts with, a power of two.
    pub const DEFAULT_FRAMES: usize = 65_536;

    /// The channel count a fresh `~buffer` starts with.
    pub const DEFAULT_CHANNELS: usize = 1;

    /// The largest frame count.
    pub const MAX_FRAMES: usize = 1 << 24;

    /// The largest channel count.
    pub const MAX_CHANNELS: usize = 32;

    /// A buffer of `frames` frames and `channels` channels, each clamped to
    /// its allowed range.
    pub fn new(frames: usize, channels: usize) -> Self {
        Buffer {
            frames: frames.clamp(1, Self::MAX_FRAMES),
            channels: channels.clamp(1, Self::MAX_CHANNELS),
            wavetable: false,
        }
    }

    /// The same buffer, with its table in the wavetable format or not.
    pub fn with_wavetable(self, wavetable: bool) -> Self {
        Buffer { wavetable, ..self }
    }

    /// The frame count, within `1..=MAX_FRAMES`.
    pub fn frames(&self) -> usize {
        self.frames.clamp(1, Self::MAX_FRAMES)
    }

    /// The channel count, within `1..=MAX_CHANNELS`.
    pub fn channels(&self) -> usize {
        self.channels.clamp(1, Self::MAX_CHANNELS)
    }

    /// True when the driver converts the table to the wavetable format.
    pub fn wavetable(&self) -> bool {
        self.wavetable
    }

    /// Set the frame count, clamped to `1..=MAX_FRAMES`. It is structural.
    pub fn set_frames(&mut self, frames: usize) {
        self.frames = frames.clamp(1, Self::MAX_FRAMES);
    }

    /// Set the channel count, clamped to `1..=MAX_CHANNELS`. It is structural.
    pub fn set_channels(&mut self, channels: usize) {
        self.channels = channels.clamp(1, Self::MAX_CHANNELS);
    }

    /// Set the table format. It is structural.
    pub fn set_wavetable(&mut self, wavetable: bool) {
        self.wavetable = wavetable;
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer::new(default_frames(), default_channels())
    }
}

impl gantz_core::Node for Buffer {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        // The table, a control input.
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        gantz_core::node::state::init_value_if_absent(ctx.vm(), path, table_state).unwrap()
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        // A non-empty list on the table input replaces the table. Anything
        // else is ignored, for example the placeholder output of a dsp node.
        // The output is a non-numeric placeholder for the inert dsp output
        // edge, see the `NodeDsp` docs.
        let expr = match ctx.inputs().first() {
            Some(Some(val)) => format!(
                "(begin \
                   (if (list? {val}) \
                       (if (empty? {val}) \
                           void \
                           (set! state \
                             (hash-insert (hash-insert state '{TABLE} {val}) '{DIRTY} #t))) \
                       void) \
                   '())"
            ),
            _ => "'()".to_string(),
        };
        gantz_core::node::parse_expr(&expr)
    }
}

impl NodeDsp for Buffer {
    fn n_dsp_inputs(&self) -> usize {
        0
    }

    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn is_buffer_source(&self) -> bool {
        true
    }

    fn ugens(&self, path: &[usize], _inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let source = BufferSource::Scratch {
            frames: self.frames(),
            wavetable: self.wavetable,
        };
        vec![b.push_buffer(path, source, self.channels())]
    }
}

impl ToNodeDsp for Buffer {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

/// True when a table arrived at the `~buffer` at `path` after the last call.
/// The call clears the flag. False when the node has no table state.
pub fn take_dirty(vm: &mut Engine, path: &[usize]) -> bool {
    let Ok(Some(SteelVal::HashMapV(map))) = gantz_core::node::state::extract_value(vm, path) else {
        return false;
    };
    if map.get(&sym(DIRTY)) != Some(&SteelVal::BoolV(true)) {
        return false;
    }
    let cleared = map.update(sym(DIRTY), SteelVal::BoolV(false));
    let _ = gantz_core::node::state::update_value(
        vm,
        path,
        SteelVal::HashMapV(Gc::new(cleared).into()),
    );
    true
}

/// The latest table of the `~buffer` at `path`. Empty when no table arrived
/// or the node has no table state.
pub fn table(vm: &Engine, path: &[usize]) -> Result<Vec<f32>, TableError> {
    let Ok(Some(SteelVal::HashMapV(map))) = gantz_core::node::state::extract_value(vm, path) else {
        return Ok(Vec::new());
    };
    let Some(SteelVal::ListV(list)) = map.get(&sym(TABLE)) else {
        return Ok(Vec::new());
    };
    list.iter()
        .enumerate()
        .map(|(index, v)| steel_num(v).map(|n| n as f32).ok_or(TableError { index }))
        .collect()
}

/// The interleaved samples of a buffer of `frames` frames and `channels`
/// channels that holds `table`. In the wavetable format when `wavetable` is
/// set. A short table is padded with zeros and a long table is cut.
pub fn table_samples(table: &[f32], frames: usize, channels: usize, wavetable: bool) -> Vec<f32> {
    let mut samples = match wavetable {
        true => plyphon::to_wavetable(table),
        false => table.to_vec(),
    };
    samples.resize(frames * channels, 0.0);
    samples
}

/// The initial VM state of a `~buffer`, an empty table that is not dirty.
fn table_state() -> SteelVal {
    let map = HashMap::new()
        .update(sym(TABLE), SteelVal::ListV(Default::default()))
        .update(sym(DIRTY), SteelVal::BoolV(false));
    SteelVal::HashMapV(Gc::new(map).into())
}

fn default_frames() -> usize {
    Buffer::DEFAULT_FRAMES
}

fn is_default_frames(frames: &usize) -> bool {
    *frames == default_frames()
}

fn default_channels() -> usize {
    Buffer::DEFAULT_CHANNELS
}

fn is_default_channels(channels: &usize) -> bool {
    *channels == default_channels()
}

fn is_false(b: &bool) -> bool {
    !b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the expr of a `~buffer` whose table input is `input_expr`, like
    /// the stateful wrapper of the codegen, on a VM with a fresh table state
    /// at `[0]`.
    fn eval_table_input(vm: &mut Engine, input_expr: &str) {
        let path = [0usize];
        let inputs = [Some(input_expr.to_string())];
        let outputs = gantz_core::node::Conns::try_from([true]).unwrap();
        let ctx = gantz_core::node::ExprCtx::new(&|_| None, &path, &inputs, &outputs);
        let node = Buffer::default();
        let expr = gantz_core::Node::expr(&node, ctx)
            .expect("expr")
            .to_pretty(80);
        let src = format!(
            "(define state (hash-ref {root} 0))
             {expr}
             (set! {root} (hash-insert {root} 0 state))",
            root = gantz_core::ROOT_STATE,
        );
        vm.run(src).expect("run table input expr");
    }

    fn test_vm() -> Engine {
        let mut vm = Engine::new_base();
        vm.register_value(gantz_core::ROOT_STATE, SteelVal::empty_hashmap());
        gantz_core::node::state::update_value(&mut vm, &[0], table_state()).unwrap();
        vm
    }

    #[test]
    fn buffer_pushes_a_scratch_binding() {
        let mut b = DspBuilder::new(1);
        Buffer::new(128, 2)
            .with_wavetable(true)
            .ugens(&[3], &[], &mut b);
        let finished = b.finish("t");
        assert_eq!(finished.buffers.len(), 1);
        let binding = &finished.buffers[0];
        let source = BufferSource::Scratch {
            frames: 128,
            wavetable: true,
        };
        assert_eq!(binding.source, source);
        assert_eq!(binding.channels, 2);
        assert_eq!(binding.node_path, vec![3]);
    }

    #[test]
    fn shape_is_clamped() {
        let buffer = Buffer::new(0, 1_000);
        assert_eq!(buffer.frames(), 1);
        assert_eq!(buffer.channels(), Buffer::MAX_CHANNELS);
    }

    #[test]
    fn default_serializes_bare() {
        assert_eq!(ron::to_string(&Buffer::default()).unwrap(), "()");
        let wavetable = Buffer::default().with_wavetable(true);
        assert_eq!(ron::to_string(&wavetable).unwrap(), "(wavetable:true)");
    }

    #[test]
    fn a_list_sets_the_table_and_the_dirty_flag() {
        let mut vm = test_vm();
        assert!(!take_dirty(&mut vm, &[0]), "a fresh table is not dirty");
        assert_eq!(table(&vm, &[0]), Ok(vec![]));
        eval_table_input(&mut vm, "(list 1 0.5 -0.5)");
        assert_eq!(table(&vm, &[0]), Ok(vec![1.0, 0.5, -0.5]));
        assert!(take_dirty(&mut vm, &[0]));
        assert!(!take_dirty(&mut vm, &[0]), "the take clears the flag");
        assert_eq!(
            table(&vm, &[0]),
            Ok(vec![1.0, 0.5, -0.5]),
            "the table stays"
        );
    }

    #[test]
    fn a_value_that_is_not_a_list_is_ignored() {
        for ignored in ["(list)", "'()", "0.5", "'sym"] {
            let mut vm = test_vm();
            eval_table_input(&mut vm, ignored);
            assert!(!take_dirty(&mut vm, &[0]), "{ignored} is ignored");
            assert_eq!(table(&vm, &[0]), Ok(vec![]), "{ignored} is ignored");
        }
    }

    #[test]
    fn a_table_element_that_is_not_a_number_is_an_error() {
        let mut vm = test_vm();
        eval_table_input(&mut vm, "(list 1 'x 2)");
        assert_eq!(table(&vm, &[0]), Err(TableError { index: 1 }));
    }

    #[test]
    fn table_samples_pads_and_cuts() {
        assert_eq!(
            table_samples(&[1.0, 2.0, 3.0], 2, 2, false),
            vec![1.0, 2.0, 3.0, 0.0]
        );
        assert_eq!(table_samples(&[1.0, 2.0, 3.0], 2, 1, false), vec![1.0, 2.0]);
    }

    #[test]
    fn table_samples_converts_to_the_wavetable_format() {
        let table = [0.0, 1.0, 0.0, -1.0];
        let samples = table_samples(&table, 8, 1, true);
        assert_eq!(samples, plyphon::to_wavetable(&table));
    }
}
