//! The generic plyphon-unit node: one node type for every descriptor-table row.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_nodetag::NodeTag;
use plyphon::synthdef::{InputRef, UnitSpec};
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, NodeRate, Signal, ToNodeDsp};
use crate::param::{control_inputs_expr, param_name, params_state, plyphon_param};
use crate::units::{In, UnitDesc, unit_desc};

/// A node wrapping one plyphon unit generator, driven entirely by its
/// [`UnitDesc`] descriptor row (see [`units`](crate::units)): the descriptor
/// declares the sockets, hybrid control params, init-only values and outputs;
/// this type provides the one `Node`/`NodeDsp` implementation shared by every
/// wrapped unit.
///
/// Hybrid param *values* live in the node's keyed VM state (see
/// [`param`](crate::param)), so editing them does not churn the graph's
/// content address. The `rate`, per-param smoothing `lags` and init-only
/// `init` values are structural (they change the derived synthdef) and live in
/// the node weight.
///
/// Deserialization validates the `unit` name against the descriptor table -
/// an unknown unit fails to reify, like an unknown node type tag.
#[derive(Clone, Debug, Serialize, Deserialize, NodeTag)]
#[tag("Unit")]
#[serde(try_from = "UnitNodeWire")]
pub struct UnitNode {
    /// The plyphon unit name (the descriptor-table key), e.g. `"LPF"`.
    unit: String,
    /// The ugen rate (`ar`/`kr`) the unit runs at.
    #[serde(default, skip_serializing_if = "crate::node::is_default")]
    rate: NodeRate,
    /// Per-hybrid-param smoothing lags in seconds, keyed by param name.
    /// Absent means `0.0` (no smoothing); entries never hold `0.0`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    lags: BTreeMap<String, f32>,
    /// Init-only structural values, keyed by name. Absent means the
    /// descriptor default; entries never hold the default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    init: BTreeMap<String, f32>,
}

/// The wire mirror of [`UnitNode`], deserialized then validated/normalised
/// via `TryFrom` (serde `try_from`).
#[derive(Deserialize)]
struct UnitNodeWire {
    unit: String,
    #[serde(default)]
    rate: NodeRate,
    #[serde(default)]
    lags: BTreeMap<String, f32>,
    #[serde(default)]
    init: BTreeMap<String, f32>,
}

/// A [`UnitNode`] failed to deserialize: an unknown unit name, or a lag/init
/// key naming no such param in the unit's descriptor.
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
        // Normalise: entries at their defaults are represented by absence, so
        // hand-authored data can't land on a non-canonical content address.
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
            rate: NodeRate::default(),
            lags: BTreeMap::new(),
            init: BTreeMap::new(),
        }
    }

    /// The node wrapping the plyphon unit of the given name, if the
    /// descriptor table covers it.
    pub fn from_unit(unit: &str) -> Option<Self> {
        unit_desc(unit).map(Self::from_desc)
    }

    /// The plyphon unit name this node wraps, e.g. `"LPF"`.
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// The node's descriptor row.
    pub fn desc(&self) -> &'static UnitDesc {
        unit_desc(&self.unit).expect("a `UnitNode`'s unit is validated at construction")
    }

    /// The ugen rate (`ar`/`kr`) the unit runs at.
    pub fn rate(&self) -> NodeRate {
        self.rate
    }

    /// Set the ugen rate (content-address affecting; structural).
    pub fn set_rate(&mut self, rate: NodeRate) {
        self.rate = rate;
    }

    /// The `name`d hybrid param's smoothing lag in seconds (`0.0` = none).
    pub fn lag(&self, name: &str) -> f32 {
        self.lags.get(name).copied().unwrap_or(0.0)
    }

    /// Set the `name`d hybrid param's smoothing lag (content-address
    /// affecting; structural - it bakes a `LagControl` into the synthdef).
    /// `0.0` removes the entry (the canonical no-lag form).
    pub fn set_lag(&mut self, name: &str, lag: f32) {
        if lag == 0.0 {
            self.lags.remove(name);
        } else {
            self.lags.insert(name.to_string(), lag);
        }
    }

    /// The `name`d init-only value (the descriptor default when unset).
    pub fn init_value(&self, name: &str) -> f32 {
        self.init
            .get(name)
            .copied()
            .or_else(|| self.desc().init_default(name))
            .unwrap_or(0.0)
    }

    /// Set the `name`d init-only value (content-address affecting;
    /// structural - it is baked into the def as a constant). The descriptor
    /// default removes the entry (the canonical form).
    pub fn set_init(&mut self, name: &str, value: f32) {
        match self.desc().init_default(name) {
            Some(default) if default.to_bits() == value.to_bits() => {
                self.init.remove(name);
            }
            Some(_) => {
                self.init.insert(name.to_string(), value);
            }
            // No such init entry: nothing to set.
            None => (),
        }
    }

    /// The Steel placeholder this node's expr evaluates to: one non-numeric
    /// value per dsp output (the multi-output expr contract).
    fn output_placeholder(&self) -> String {
        match self.desc().outputs.len() {
            1 => "'()".to_string(),
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
        // Hybrid param values live in (keyed) VM state.
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
        // Steel-inert bar the hybrid inputs: a connected *number* is written
        // into the param's keyed state (the audio driver applies it via
        // `set_control`), while a dsp source's non-numeric placeholder is
        // ignored by the `number?` guard. See `control_inputs_expr`.
        let hybrids: Vec<(usize, &str)> = self.desc().hybrid_sockets().collect();
        control_inputs_expr(&ctx, &hybrids, &self.output_placeholder())
    }
}

/// Channel `c`'s wire of a connected input: mono broadcasts its only channel
/// across the whole group; a narrower multi-channel signal contributes
/// silence past its width (the [`Signal`] group conventions).
fn channel_select(signal: &Signal, c: usize) -> InputRef {
    match signal.width() {
        1 => signal.channel(0).expect("a `Signal` is never empty"),
        _ => signal.channel(c).unwrap_or(InputRef::Constant(0.0)),
    }
}

/// How one plyphon input of the emitted units is fed, resolved once per node
/// (params are shared across the channel group; wires select per channel).
enum Feed {
    /// A connected socket's signal.
    Wire(Signal),
    /// An unconnected hybrid input's shared control param.
    Param(u32),
    /// A constant: an unconnected pure signal input (silence), a baked value
    /// or an init-only value.
    Const(f32),
}

impl NodeDsp for UnitNode {
    fn n_dsp_inputs(&self) -> usize {
        // Every socket is dsp-capable (pure signal or hybrid).
        self.desc().n_sockets()
    }

    fn n_dsp_outputs(&self) -> usize {
        self.desc().outputs.len()
    }

    fn ugens(&self, path: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let desc = self.desc();
        // The channel-group width: one unit per channel of the widest
        // connected input (or a single unit when nothing is connected).
        let width = inputs
            .iter()
            .flatten()
            .map(Signal::width)
            .max()
            .unwrap_or(1);
        // Resolve each plyphon input's feed once: connected sockets keep
        // their signal, unconnected hybrids get one shared control param
        // (broadcast across the group), everything else is a constant.
        let mut sockets = 0..;
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
                In::Baked(v) => Feed::Const(*v),
                In::Init { name, default, .. } => {
                    Feed::Const(self.init.get(*name).copied().unwrap_or(*default))
                }
            })
            .collect();
        // One unit per channel, then output port `j` groups every channel
        // unit's `j`th output.
        let units: Vec<u32> = (0..width)
            .map(|c| {
                let ins = feeds
                    .iter()
                    .map(|feed| match feed {
                        Feed::Wire(signal) => channel_select(signal, c),
                        Feed::Param(ix) => InputRef::Param(*ix),
                        Feed::Const(v) => InputRef::Constant(*v),
                    })
                    .collect();
                b.push_unit(UnitSpec::new(
                    desc.unit,
                    self.rate.to_plyphon(),
                    ins,
                    desc.outputs.len(),
                ))
            })
            .collect();
        (0..desc.outputs.len())
            .map(|output| {
                units
                    .iter()
                    .map(|&unit| InputRef::Unit {
                        unit,
                        output: output as u32,
                    })
                    .collect()
            })
            .collect()
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
        // "unset": deserialization must land on the canonical node.
        let node: UnitNode =
            ron::from_str(r#"(unit: "CombC", lags: {"delay": 0.0}, init: {"maxdelay": 0.2})"#)
                .expect("deserialize");
        assert_eq!(node, UnitNode::from_unit("CombC").expect("CombC row"));
        // And the setters keep the same invariant.
        let mut node = UnitNode::from_unit("CombC").expect("CombC row");
        node.set_lag("delay", 0.02);
        node.set_lag("delay", 0.0);
        node.set_init("maxdelay", 0.5);
        node.set_init("maxdelay", 0.2);
        assert_eq!(node, UnitNode::from_unit("CombC").expect("CombC row"));
    }
}
