//! Builtin specs for the DSP node set.

use crate::node::{Buffer, Bus, Envgen, Out, Pack, Sample, ScopeOut, Sum, UnitNode, Unpack};
use gantz_core::Builtin;

/// Builtin specs for the DSP node set, the bespoke nodes plus one entry per
/// [`crate::units`] descriptor-table row.
pub fn builtins() -> Vec<Builtin> {
    let bespoke = [
        Builtin::new("~buffer", &Buffer::default()),
        Builtin::new("~bus", &Bus::default()),
        Builtin::new("~envgen", &Envgen::default()),
        Builtin::new("~out", &Out::default()),
        Builtin::new("~pack", &Pack::default()),
        Builtin::new("~sample", &Sample::default()),
        Builtin::new("~scopeout", &ScopeOut::default()),
        Builtin::new("~sum", &Sum::default()),
        Builtin::new("~unpack", &Unpack::default()),
    ];
    bespoke
        .into_iter()
        .chain(
            crate::units::UNITS
                .iter()
                .map(|desc| Builtin::new(desc.keyword, &UnitNode::from_desc(desc))),
        )
        .collect()
}
