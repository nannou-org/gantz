//! Builtin specs for the DSP node set.

use crate::node::{Bus, Out, Pack, PlayBuf, ScopeOut, Sum, UnitNode, Unpack};
use gantz_core::Builtin;

/// Builtin specs for the DSP node set: the bespoke nodes plus one entry per
/// descriptor-table row (see [`crate::units`]).
pub fn builtins() -> Vec<Builtin> {
    let bespoke = [
        Builtin::new("~bus", &Bus::default()),
        Builtin::new("~out", &Out::default()),
        Builtin::new("~pack", &Pack::default()),
        Builtin::new("~playbuf", &PlayBuf::default()),
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
