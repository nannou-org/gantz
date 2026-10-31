//! The `~buffer` scratch buffer source node.
//!
//! A `~buffer` can also hold a table. A list of numbers on its control input
//! is kept in the node's VM state as `{ table, dirty }`. The DSP runtime
//! writes the table into the buffer when `dirty` is set, and into each new
//! buffer of the node. See [`take_dirty`], [`table`] and [`table_samples`].

use gantz_ca::Datum;
use gantz_nodedef::{ExprArgs, Meta, View};
use gantz_nodetag::NodeTag;
use gantz_ui::{self as ui, Edit, Element};
use serde::{Deserialize, Serialize};

use crate::dsp::{BufferSource, DspBuilder, NodeDsp, Signal, ToNodeDsp};
use crate::param::map;
use crate::view::{doc, edit_count, edit_dialer, row};

/// A zeroed scratch buffer as a buffer source. It emits the bufnum wire of
/// the buffer, for example for a `~recordbuf` that writes it and a
/// `~playbuf` that reads it.
///
/// The DSP runtime allocates one buffer per open head and node path, so
/// each instance of a nested graph gets its own buffer. The buffer keeps its
/// contents when the synths that use it respawn. A change of `frames`,
/// `channels` or `wavetable` gives a new buffer.
///
/// A list of numbers on the control input is a table. The runtime writes it
/// into the buffer as interleaved samples and sets the rest to zero. With
/// `wavetable` set, the runtime first converts the list to the format that
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

/// The state key that is true when a table arrived after the runtime last
/// took it.
const DIRTY: &str = "dirty";

/// The edits of the buffer's shape and table format.
const FRAMES: &str = "frames";
const CHANNELS: &str = "channels";
const WAVETABLE: &str = "wavetable";

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

    /// True when the runtime converts the table to the wavetable format.
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

impl gantz_nodedef::NodeDef for Buffer {
    const TAG: &'static str = <Self as NodeTag>::TAG;

    fn meta(&self) -> Meta {
        Meta {
            name: "~buffer".into(),
            description:
                "A scratch buffer for units that write and read a buffer, or a table from a list"
                    .into(),
            // The table, a control input.
            inputs: 1,
            outputs: 1,
            stateful: true,
            ..Meta::default()
        }
    }

    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
        // A non-empty list on the table input replaces the table. Anything
        // else is ignored, for example the placeholder output of a dsp node.
        // The output is a non-numeric placeholder for the inert dsp output
        // edge, see the `NodeDsp` docs.
        Ok(match ctx.inputs.first() {
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
        })
    }

    fn init_state(&self) -> Option<Datum> {
        Some(table_state())
    }

    fn view(&self) -> View {
        // The shape and the table format are structural. A change gives a
        // new buffer.
        let frames = ui::Dialer {
            min: Some(1.0),
            max: Some(Self::MAX_FRAMES as f64),
            step: Some(64.0),
            precision: Some(0),
            label: Some("frames".into()),
            ..edit_dialer(FRAMES, self.frames() as f64)
        };
        let channels = ui::Dialer {
            min: Some(1.0),
            max: Some(Self::MAX_CHANNELS as f64),
            step: Some(0.05),
            precision: Some(0),
            ..edit_dialer(CHANNELS, self.channels() as f64)
        };
        let wavetable = ui::Toggle {
            on: Some(Edit(WAVETABLE.into())),
            value: self.wavetable,
            tip: Some("convert the table to the wavetable format of `~osc` and `~cosc`".into()),
            ..ui::Toggle::default()
        };
        View {
            inputs: vec![doc("list", "a table of numbers to write into the buffer")],
            outputs: vec![doc("buffer", "the buffer, for a buffer socket")],
            rows: vec![
                row("frames", Element::Dialer(frames)),
                row("channels", Element::Dialer(channels)),
                row("wavetable", Element::Toggle(wavetable)),
            ]
            .into(),
            ..View::default()
        }
    }

    fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
        let mut buffer = self.clone();
        match edit {
            FRAMES => buffer.set_frames(edit_count(value, [1, Self::MAX_FRAMES])?),
            CHANNELS => buffer.set_channels(edit_count(value, [1, Self::MAX_CHANNELS])?),
            WAVETABLE => buffer.set_wavetable(value.as_bool()?),
            _ => return None,
        }
        Some(buffer)
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

/// The `~buffer` node state `state` with its `dirty` flag cleared, when a
/// table arrived at the node since the flag was last cleared. `None` when no
/// table arrived, or the state is not a `~buffer`'s.
pub fn take_dirty(state: &Datum) -> Option<Datum> {
    let Datum::Map(entries) = state else {
        return None;
    };
    if state.get(DIRTY) != Some(&Datum::Bool(true)) {
        return None;
    }
    let entries = entries.iter().map(|(k, v)| match k == DIRTY {
        true => (k.clone(), Datum::Bool(false)),
        false => (k.clone(), v.clone()),
    });
    Some(map(entries.collect()))
}

/// The latest table in the `~buffer` node state `state`. Empty when no
/// table arrived or the state is not a `~buffer`'s.
pub fn table(state: &Datum) -> Result<Vec<f32>, TableError> {
    let Some(Datum::Seq(list)) = state.get(TABLE) else {
        return Ok(Vec::new());
    };
    list.iter()
        .enumerate()
        .map(|(index, v)| v.as_f64().map(|n| n as f32).ok_or(TableError { index }))
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

/// The initial state of a `~buffer`, an empty table that is not dirty.
fn table_state() -> Datum {
    map(vec![
        (TABLE.to_string(), Datum::Seq(vec![])),
        (DIRTY.to_string(), Datum::Bool(false)),
    ])
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
    use gantz_core::steel::steel_vm::engine::Engine;

    /// The table state of the `~buffer` at `[0]` in `vm`.
    fn state(vm: &Engine) -> Datum {
        crate::state_value(vm, &[0]).expect("the table state")
    }

    /// Run the expr of a `~buffer` whose table input is `input_expr`, like
    /// the stateful wrapper of the codegen, on a VM with a fresh table state
    /// at `[0]`.
    fn eval_table_input(vm: &mut Engine, input_expr: &str) {
        let path = [0];
        let inputs = [Some(input_expr)];
        let ctx = gantz_nodedef::ExprArgs {
            inputs: &inputs,
            outputs: &[true],
            path: &path,
            args: gantz_core::ARGS,
        };
        let expr = gantz_nodedef::NodeDef::expr(&Buffer::default(), ctx).expect("expr");
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
        let root = gantz_core::steel::SteelVal::empty_hashmap();
        vm.register_value(gantz_core::ROOT_STATE, root);
        let state = gantz_core::datum::to_steel(&table_state());
        gantz_core::node::state::update_value(&mut vm, &[0], state).unwrap();
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
        assert_eq!(take_dirty(&state(&vm)), None, "a fresh table is not dirty");
        assert_eq!(table(&state(&vm)), Ok(vec![]));
        eval_table_input(&mut vm, "(list 1 0.5 -0.5)");
        let dirty = state(&vm);
        assert_eq!(table(&dirty), Ok(vec![1.0, 0.5, -0.5]));
        let taken = take_dirty(&dirty).expect("a new table is dirty");
        assert_eq!(take_dirty(&taken), None, "the take clears the flag");
        assert_eq!(table(&taken), Ok(vec![1.0, 0.5, -0.5]), "the table stays");
    }

    #[test]
    fn a_value_that_is_not_a_list_is_ignored() {
        for ignored in ["(list)", "'()", "0.5", "'sym"] {
            let mut vm = test_vm();
            eval_table_input(&mut vm, ignored);
            assert_eq!(take_dirty(&state(&vm)), None, "{ignored} is ignored");
            assert_eq!(table(&state(&vm)), Ok(vec![]), "{ignored} is ignored");
        }
    }

    #[test]
    fn a_table_element_that_is_not_a_number_is_an_error() {
        let mut vm = test_vm();
        eval_table_input(&mut vm, "(list 1 'x 2)");
        assert_eq!(table(&state(&vm)), Err(TableError { index: 1 }));
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
