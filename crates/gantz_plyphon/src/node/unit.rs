//! The generic plyphon-unit node: one node type for every descriptor-table row.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_nodetag::NodeTag;
use plyphon::synthdef::{InputRef, UnitSpec};
use serde::{Deserialize, Serialize};

use crate::dsp::{BufferAccess, DspBuilder, NodeDsp, NodeRate, Signal, ToNodeDsp};
use crate::param::{control_inputs_expr, param_name, params_state, plyphon_param};
use crate::units::{Emit, In, UnitDesc, UnitRate, unit_desc};

/// A node wrapping one plyphon unit generator, driven entirely by its
/// [`UnitDesc`] descriptor row, see [`units`](crate::units). The descriptor
/// declares the sockets, hybrid control params, init-only values and outputs.
/// This type provides the one `Node`/`NodeDsp` implementation shared by every
/// wrapped unit.
///
/// Hybrid param values live in the node's keyed VM state, see
/// [`param`](crate::param), so editing them does not churn the graph's
/// content address. The `rate`, per-param smoothing `lags` and init-only
/// `init` values are structural, since they change the derived synthdef, and
/// live in the node weight.
///
/// Deserialization validates the `unit` name against the descriptor table.
/// An unknown unit fails to reify, like an unknown node type tag. A `rate`
/// that a [`UnitRate::Fixed`] row does not allow also fails.
#[derive(Clone, Debug, Serialize, Deserialize, NodeTag)]
#[tag("Unit")]
#[serde(try_from = "UnitNodeWire")]
pub struct UnitNode {
    /// The plyphon unit name, the descriptor-table key, for example `"LPF"`.
    unit: String,
    /// The ugen rate, `ar` or `kr`. `None` means the row's
    /// [`default_rate`](UnitDesc::default_rate). An entry never holds the
    /// default rate. A [`UnitRate::Fixed`] row never has an entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rate: Option<NodeRate>,
    /// Per-hybrid-param smoothing lags in seconds, keyed by param name.
    /// Absent means `0.0`, no smoothing. Entries never hold `0.0`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    lags: BTreeMap<String, f32>,
    /// Init-only structural values, keyed by name. Absent means the
    /// descriptor default. Entries never hold the default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    init: BTreeMap<String, f32>,
}

/// The wire mirror of [`UnitNode`], deserialized then validated and
/// normalised via serde's `try_from`.
#[derive(Deserialize)]
struct UnitNodeWire {
    unit: String,
    #[serde(default)]
    rate: Option<NodeRate>,
    #[serde(default)]
    lags: BTreeMap<String, f32>,
    #[serde(default)]
    init: BTreeMap<String, f32>,
}

/// A [`UnitNode`] failed to deserialize. The cause is an unknown unit name,
/// a lag or init key for a param that the descriptor does not have, or a
/// rate that a fixed-rate row does not allow.
#[derive(Debug)]
pub struct InvalidUnitNode(String);

impl fmt::Display for InvalidUnitNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for InvalidUnitNode {}

impl TryFrom<UnitNodeWire> for UnitNode {
    type Error = InvalidUnitNode;

    fn try_from(wire: UnitNodeWire) -> Result<Self, Self::Error> {
        let UnitNodeWire {
            unit,
            rate,
            lags,
            init,
        } = wire;
        let Some(desc) = unit_desc(&unit) else {
            return Err(InvalidUnitNode(format!("unknown plyphon unit `{unit}`")));
        };
        for name in lags.keys() {
            if !desc.hybrid_params().any(|(n, _)| n == name) {
                return Err(InvalidUnitNode(format!(
                    "unit `{unit}` has no hybrid param `{name}` to lag"
                )));
            }
        }
        for name in init.keys() {
            if desc.init_default(name).is_none() {
                return Err(InvalidUnitNode(format!(
                    "unit `{unit}` has no init value `{name}`"
                )));
            }
        }
        if let (UnitRate::Fixed(fixed), Some(rate)) = (desc.rate, rate) {
            if rate != fixed {
                return Err(InvalidUnitNode(format!(
                    "unit `{unit}` runs at `{}` only, got `{}`",
                    fixed.token(),
                    rate.token()
                )));
            }
        }
        // Normalise. Entries at their defaults are represented by absence, so
        // hand-authored data cannot land on a non-canonical content address.
        let rate = rate.filter(|r| *r != desc.default_rate());
        let lags = lags.into_iter().filter(|(_, lag)| *lag != 0.0).collect();
        let init = init
            .into_iter()
            .filter(|(name, v)| {
                let default = desc.init_default(name).expect("validated above");
                v.to_bits() != default.to_bits()
            })
            .collect();
        Ok(UnitNode {
            unit,
            rate,
            lags,
            init,
        })
    }
}

impl UnitNode {
    /// The node for the given descriptor row, at its defaults.
    pub fn from_desc(desc: &'static UnitDesc) -> Self {
        UnitNode {
            unit: desc.unit.to_string(),
            rate: None,
            lags: BTreeMap::new(),
            init: BTreeMap::new(),
        }
    }

    /// The node wrapping the plyphon unit of the given name, if the
    /// descriptor table covers it.
    pub fn from_unit(unit: &str) -> Option<Self> {
        unit_desc(unit).map(Self::from_desc)
    }

    /// The plyphon unit name this node wraps, for example `"LPF"`.
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// The node's descriptor row.
    pub fn desc(&self) -> &'static UnitDesc {
        unit_desc(&self.unit).expect("a `UnitNode`'s unit is validated at construction")
    }

    /// The ugen rate, `ar` or `kr`, the unit runs at.
    pub fn rate(&self) -> NodeRate {
        self.rate.unwrap_or_else(|| self.desc().default_rate())
    }

    /// Set the ugen rate. It is structural and affects the content address.
    /// The default rate of the row removes the entry, which is the canonical
    /// form. A fixed-rate row ignores the call.
    pub fn set_rate(&mut self, rate: NodeRate) {
        let desc = self.desc();
        self.rate = match desc.rate {
            UnitRate::Fixed(_) => None,
            UnitRate::Any => Some(rate).filter(|r| *r != desc.default_rate()),
        };
    }

    /// The `name`d hybrid param's smoothing lag in seconds. `0.0` is none.
    pub fn lag(&self, name: &str) -> f32 {
        self.lags.get(name).copied().unwrap_or(0.0)
    }

    /// Set the `name`d hybrid param's smoothing lag. It is structural and
    /// affects the content address, since it bakes a `LagControl` into the
    /// synthdef. `0.0` removes the entry, the canonical no-lag form.
    pub fn set_lag(&mut self, name: &str, lag: f32) {
        if lag == 0.0 {
            self.lags.remove(name);
        } else {
            self.lags.insert(name.to_string(), lag);
        }
    }

    /// The `name`d init-only value, or the descriptor default when unset.
    pub fn init_value(&self, name: &str) -> f32 {
        self.init
            .get(name)
            .copied()
            .or_else(|| self.desc().init_default(name))
            .unwrap_or(0.0)
    }

    /// Set the `name`d init-only value. It is structural and affects the
    /// content address, since it is baked into the def as a constant. The
    /// descriptor default removes the entry, the canonical form.
    pub fn set_init(&mut self, name: &str, value: f32) {
        match self.desc().init_default(name) {
            Some(default) if default.to_bits() == value.to_bits() => {
                self.init.remove(name);
            }
            Some(_) => {
                self.init.insert(name.to_string(), value);
            }
            // No such init entry, nothing to set.
            None => (),
        }
    }

    /// The init-only channel count `name`, rounded and clamped to `1..=max`.
    fn channels_value(&self, name: &str, max: usize) -> usize {
        (self.init_value(name).round().max(1.0) as usize).min(max.max(1))
    }

    /// The Steel placeholder this node's expr evaluates to, one non-numeric
    /// value per dsp output per the multi-output expr contract. A node with
    /// no outputs still evaluates to one inert value.
    fn output_placeholder(&self) -> String {
        match self.desc().outputs.len() {
            0 | 1 => "'()".to_string(),
            n => format!("(list {})", vec!["'()"; n].join(" ")),
        }
    }
}

impl PartialEq for UnitNode {
    fn eq(&self, other: &Self) -> bool {
        fn bits(map: &BTreeMap<String, f32>) -> impl Iterator<Item = (&String, u32)> {
            map.iter().map(|(k, v)| (k, v.to_bits()))
        }
        self.unit == other.unit
            && self.rate == other.rate
            && bits(&self.lags).eq(bits(&other.lags))
            && bits(&self.init).eq(bits(&other.init))
    }
}

impl Eq for UnitNode {}

impl Hash for UnitNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.unit.hash(state);
        self.rate.hash(state);
        for (name, v) in &self.lags {
            name.hash(state);
            v.to_bits().hash(state);
        }
        for (name, v) in &self.init {
            name.hash(state);
            v.to_bits().hash(state);
        }
    }
}

impl gantz_core::Node for UnitNode {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        self.desc().n_sockets()
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        self.desc().outputs.len()
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        // Hybrid param values live in keyed VM state.
        self.desc().hybrid_params().next().is_some()
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let defaults: Vec<(&str, f64)> = self
            .desc()
            .hybrid_params()
            .map(|(name, default)| (name, default as f64))
            .collect();
        if defaults.is_empty() {
            return;
        }
        let path = ctx.path();
        gantz_core::node::state::init_value_if_absent(ctx.vm(), path, || params_state(&defaults))
            .unwrap()
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        // Steel-inert bar the hybrid inputs. A connected number is written
        // into the param's keyed state and the audio driver applies it via
        // `set_control`. The `number?` guard ignores a dsp source's
        // non-numeric placeholder. See `control_inputs_expr`.
        let hybrids: Vec<(usize, &str)> = self.desc().hybrid_sockets().collect();
        control_inputs_expr(&ctx, &hybrids, &self.output_placeholder())
    }
}

/// Channel `c`'s wire of a connected input. Mono broadcasts its only channel
/// across the whole group. A narrower multi-channel signal contributes
/// silence past its width, per the [`Signal`] group conventions.
fn channel_select(signal: &Signal, c: usize) -> InputRef {
    match signal.width() {
        1 => signal.channel(0).expect("a `Signal` is never empty"),
        _ => signal.channel(c).unwrap_or(InputRef::Constant(0.0)),
    }
}

/// How one plyphon input of the emitted units is fed, resolved once per node.
/// Params are shared across the channel group. Wires select per channel.
enum Feed {
    /// A connected socket's signal.
    Wire(Signal),
    /// An unconnected hybrid input's shared control param.
    Param(u32),
    /// A constant. Either an unconnected pure signal input as silence, an
    /// unresolved buffer as `-1`, a baked value or an init-only value.
    Const(f32),
    /// A group socket's signal, `None` when unconnected. It feeds one
    /// trailing input per channel.
    Group(Option<Signal>),
}

/// A buffer socket that resolved to a buffer source.
struct ResolvedBuffer {
    /// The [`In::Buffer`] entry's name.
    name: &'static str,
    /// The bufnum wire.
    bufnum: InputRef,
    /// Whether the unit writes the buffer.
    access: BufferAccess,
    /// The buffer's channel count.
    channels: usize,
}

impl NodeDsp for UnitNode {
    fn n_dsp_inputs(&self) -> usize {
        // Every socket is dsp-capable, pure signal, hybrid, buffer or group.
        self.desc().n_sockets()
    }

    fn n_dsp_outputs(&self) -> usize {
        self.desc().outputs.len()
    }

    fn is_buffer_input(&self, input: usize) -> bool {
        self.desc().is_buffer_socket(input)
    }

    fn is_writer(&self) -> bool {
        self.desc().emit == Emit::Sink
    }

    fn ugens(&self, path: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let desc = self.desc();
        // Resolve each plyphon input's feed once. Connected sockets keep their
        // signal. Unconnected hybrids get one shared control param, broadcast
        // across the group. A buffer socket gets its source's bufnum, or `-1`.
        // Everything else is a constant.
        let mut sockets = 0..;
        let mut buffers: Vec<ResolvedBuffer> = Vec::new();
        let feeds: Vec<Feed> = desc
            .inputs
            .iter()
            .map(|entry| match entry {
                In::Signal { .. } => {
                    let socket = sockets.next().expect("infinite range");
                    match inputs.get(socket).cloned().flatten() {
                        Some(signal) => Feed::Wire(signal),
                        None => Feed::Const(0.0),
                    }
                }
                In::Param { name, default, .. } => {
                    let socket = sockets.next().expect("infinite range");
                    match inputs.get(socket).cloned().flatten() {
                        Some(signal) => Feed::Wire(signal),
                        None => {
                            let param =
                                plyphon_param(param_name(path, name), *default, self.lag(name));
                            Feed::Param(b.push_param_keyed(path, name, param))
                        }
                    }
                }
                In::Buffer { name, access, .. } => {
                    let socket = sockets.next().expect("infinite range");
                    let input = inputs.get(socket).and_then(Option::as_ref);
                    match b.buffer_input(input, *access) {
                        Some((bufnum, binding)) => {
                            buffers.push(ResolvedBuffer {
                                name,
                                bufnum,
                                access: *access,
                                channels: binding.channels,
                            });
                            Feed::Wire(Signal::mono(bufnum))
                        }
                        None => Feed::Const(-1.0),
                    }
                }
                In::Group { .. } => {
                    let socket = sockets.next().expect("infinite range");
                    Feed::Group(inputs.get(socket).cloned().flatten())
                }
                In::Baked(v) => Feed::Const(*v),
                In::Init { name, default, .. } => {
                    Feed::Const(self.init.get(*name).copied().unwrap_or(*default))
                }
            })
            .collect();
        let buffer = |name: &str| buffers.iter().find(|r| r.name == name);
        // A group feeds as many channels as the buffer it writes.
        let write_channels = buffers
            .iter()
            .find(|r| r.access == BufferAccess::Write)
            .map(|r| r.channels);
        // The input to scale by its buffer's rate, and that buffer's bufnum.
        let scale = desc.rate_scale.and_then(|rs| {
            let ix = desc
                .inputs
                .iter()
                .position(|e| e.name() == Some(rs.input))?;
            Some((ix, buffer(rs.buffer)?.bufnum))
        });
        // The inputs of the unit for channel `c`, in plyphon order.
        let unit_inputs = |b: &mut DspBuilder, c: usize| -> Vec<InputRef> {
            let mut ins = Vec::with_capacity(feeds.len());
            for (ix, feed) in feeds.iter().enumerate() {
                let input = match feed {
                    Feed::Wire(signal) => channel_select(signal, c),
                    Feed::Param(p) => InputRef::Param(*p),
                    Feed::Const(v) => InputRef::Constant(*v),
                    Feed::Group(signal) => {
                        let width = write_channels
                            .or(signal.as_ref().map(Signal::width))
                            .unwrap_or(1);
                        ins.extend((0..width).map(|ch| match signal {
                            Some(signal) => channel_select(signal, ch),
                            None => InputRef::Constant(0.0),
                        }));
                        continue;
                    }
                };
                ins.push(match scale {
                    Some((six, bufnum)) if six == ix => b.rate_scaled(bufnum, input),
                    _ => input,
                });
            }
            ins
        };
        let spec = |inputs: Vec<InputRef>, num_outputs: usize| UnitSpec {
            name: desc.emitted_unit().to_string(),
            rate: self.rate().to_plyphon(),
            inputs,
            num_outputs,
            special_index: desc.special_index(),
        };
        let unit_outputs = |unit: u32, n: usize| -> Signal {
            (0..n as u32)
                .map(|output| InputRef::Unit { unit, output })
                .collect()
        };
        match desc.emit {
            Emit::Expand => {
                // One unit per channel of the widest connected input, or a
                // single unit when nothing is connected. Output port `j`
                // groups every channel unit's `j`th output.
                let width = inputs
                    .iter()
                    .flatten()
                    .map(Signal::width)
                    .max()
                    .unwrap_or(1);
                let units: Vec<u32> = (0..width)
                    .map(|c| {
                        let ins = unit_inputs(b, c);
                        b.push_unit(spec(ins, desc.outputs.len()))
                    })
                    .collect();
                (0..desc.outputs.len() as u32)
                    .map(|output| {
                        units
                            .iter()
                            .map(|&unit| InputRef::Unit { unit, output })
                            .collect()
                    })
                    .collect()
            }
            Emit::Single => {
                let ins = unit_inputs(b, 0);
                let unit = b.push_unit(spec(ins, desc.outputs.len()));
                (0..desc.outputs.len() as u32)
                    .map(|output| Signal::mono(InputRef::Unit { unit, output }))
                    .collect()
            }
            Emit::BufferChannels { socket } => {
                let n = buffer(socket).map_or(1, |r| r.channels);
                let ins = unit_inputs(b, 0);
                let unit = b.push_unit(spec(ins, n));
                vec![unit_outputs(unit, n)]
            }
            Emit::InitChannels { name, max, .. } => {
                let n = self.channels_value(name, max);
                let ins = unit_inputs(b, 0);
                let unit = b.push_unit(spec(ins, n));
                vec![unit_outputs(unit, n)]
            }
            Emit::Sink => {
                let ins = unit_inputs(b, 0);
                b.push_unit(spec(ins, 1));
                vec![]
            }
        }
    }
}

impl ToNodeDsp for UnitNode {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::datum::{Datum, from_datum, to_datum};

    #[test]
    fn serde_round_trips() {
        let mut node = UnitNode::from_unit("CombC").expect("CombC row");
        node.set_rate(NodeRate::Control);
        node.set_lag("delay", 0.02);
        node.set_init("maxdelay", 0.5);
        let ron = ron::to_string(&node).expect("serialize");
        let back: UnitNode = ron::from_str(&ron).expect("deserialize");
        assert_eq!(node, back);
    }

    #[test]
    fn unknown_unit_fails_to_deserialize() {
        let err = ron::from_str::<UnitNode>(r#"(unit: "NoSuchUnit")"#).unwrap_err();
        assert!(err.to_string().contains("unknown plyphon unit"), "{err}");
    }

    #[test]
    fn unknown_lag_and_init_keys_fail_to_deserialize() {
        assert!(ron::from_str::<UnitNode>(r#"(unit: "LPF", lags: {"nope": 0.1})"#).is_err());
        assert!(ron::from_str::<UnitNode>(r#"(unit: "LPF", init: {"nope": 0.1})"#).is_err());
    }

    #[test]
    fn defaulted_entries_normalise_to_absence() {
        // A zero lag and a default init value are non-canonical spellings of
        // unset. Deserialization must land on the canonical node.
        let node: UnitNode =
            ron::from_str(r#"(unit: "CombC", lags: {"delay": 0.0}, init: {"maxdelay": 0.2})"#)
                .expect("deserialize");
        assert_eq!(node, UnitNode::from_unit("CombC").expect("CombC row"));
        // The setters keep the same invariant.
        let mut node = UnitNode::from_unit("CombC").expect("CombC row");
        node.set_lag("delay", 0.02);
        node.set_lag("delay", 0.0);
        node.set_init("maxdelay", 0.5);
        node.set_init("maxdelay", 0.2);
        assert_eq!(node, UnitNode::from_unit("CombC").expect("CombC row"));
        // An explicit default rate is also a non-canonical form.
        let node: UnitNode = from_datum(unit_datum("CombC", Some("ar"))).expect("deserialize");
        assert_eq!(node, UnitNode::from_unit("CombC").expect("CombC row"));
        let mut node = UnitNode::from_unit("CombC").expect("CombC row");
        node.set_rate(NodeRate::Control);
        node.set_rate(NodeRate::Audio);
        assert_eq!(node, UnitNode::from_unit("CombC").expect("CombC row"));
    }

    /// A `Unit` datum in the content-address form, with an optional `rate`.
    fn unit_datum(unit: &str, rate: Option<&str>) -> Datum {
        let mut fields = vec![("unit".to_string(), Datum::Str(unit.to_string()))];
        if let Some(rate) = rate {
            fields.push(("rate".to_string(), Datum::Str(rate.to_string())));
        }
        Datum::Map(fields)
    }

    /// The content-address form of the rate must not change, because the
    /// address includes it. A default rate is absent. A control rate is
    /// `"kr"`.
    #[test]
    fn rate_wire_form_is_stable() {
        let mut node = UnitNode::from_unit("SinOsc").expect("SinOsc row");
        assert_eq!(to_datum(&node).unwrap(), unit_datum("SinOsc", None));
        node.set_rate(NodeRate::Control);
        assert_eq!(to_datum(&node).unwrap(), unit_datum("SinOsc", Some("kr")));
        // A fixed-rate row serializes bare at its fixed rate.
        let node = UnitNode::from_unit("A2K").expect("A2K row");
        assert_eq!(node.rate(), NodeRate::Control);
        assert_eq!(to_datum(&node).unwrap(), unit_datum("A2K", None));
    }

    #[test]
    fn fixed_rate_rows_reify_bare_and_reject_the_other_rate() {
        let a2k = UnitNode::from_unit("A2K").expect("A2K row");
        // An explicit fixed rate is a non-canonical form of no rate.
        let node: UnitNode = from_datum(unit_datum("A2K", Some("kr"))).expect("deserialize");
        assert_eq!(node, a2k);
        let err = from_datum::<UnitNode>(unit_datum("A2K", Some("ar"))).unwrap_err();
        assert!(err.to_string().contains("runs at `kr` only"), "{err}");
        // The setter does nothing.
        let mut node = a2k.clone();
        node.set_rate(NodeRate::Audio);
        assert_eq!(node, a2k);
        assert_eq!(node.rate(), NodeRate::Control);
    }
}
