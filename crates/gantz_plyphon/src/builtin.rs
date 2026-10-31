//! Builtin specs for the DSP node set.

use crate::node::{Buffer, Bus, Envgen, Out, Pack, Sample, ScopeOut, Sum, UnitNode, Unpack};
use gantz_core::Builtin;
use gantz_nodedef::NodeDef;

/// Builtin specs for the DSP node set, the bespoke nodes plus one entry per
/// [`crate::units`] descriptor-table row.
pub fn builtins() -> Vec<Builtin> {
    let bespoke = [
        builtin("~buffer", &Buffer::default()),
        builtin("~bus", &Bus::default()),
        builtin("~envgen", &Envgen::default()),
        builtin("~out", &Out::default()),
        builtin("~pack", &Pack::default()),
        builtin("~sample", &Sample::default()),
        builtin("~scopeout", &ScopeOut::default()),
        builtin("~sum", &Sum::default()),
        builtin("~unpack", &Unpack::default()),
    ];
    let units = crate::units::UNITS
        .iter()
        .map(|desc| builtin(desc.keyword, &UnitNode::from_desc(desc)));
    bespoke.into_iter().chain(units).collect()
}

/// The builtin `name` of the default node `node`.
///
/// Panics if the node fails to erase. A builtin that cannot erase is a
/// node-set composition error, which startup or tests catch.
fn builtin<D: NodeDef>(name: &'static str, node: &D) -> Builtin {
    let node = gantz_nodedef::node_data(node)
        .unwrap_or_else(|e| panic!("builtin `{name}` failed to erase: {e}"));
    Builtin { name, node }
}
