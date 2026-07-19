//! Builtin specs for the DSP node set.

use crate::node::{Bus, Lag, Out, Pack, PlayBuf, ScopeOut, SinOsc, Sum, Unpack};
use gantz_core::Builtin;

/// Builtin specs for the DSP node set.
pub fn builtins() -> Vec<Builtin> {
    vec![
        Builtin::new("~bus", &Bus::default()),
        Builtin::new("~lag", &Lag::default()),
        Builtin::new("~out", &Out::default()),
        Builtin::new("~pack", &Pack::default()),
        Builtin::new("~playbuf", &PlayBuf::default()),
        Builtin::new("~scopeout", &ScopeOut::default()),
        Builtin::new("~sinosc", &SinOsc::default()),
        Builtin::new("~sum", &Sum::default()),
        Builtin::new("~unpack", &Unpack::default()),
    ]
}
