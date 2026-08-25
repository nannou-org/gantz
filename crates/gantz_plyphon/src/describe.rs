//! A readable text rendering of a resolved DSP program, for diagnostics
//! surfacing (the GUI's "DSP" pane).

use std::fmt::Write;

use plyphon::Rate;
use plyphon::synthdef::{InputRef, UnitSpec};

use crate::instance::{BusKey, ResolvedBus, ResolvedPart};

/// Render a head's resolved DSP program - the parts the audio driver spawns,
/// in part order - as readable text.
///
/// Per part: the def name and identity, its params (with bound node paths),
/// its units, its monitors, the buses it writes and reads, and the width/rate
/// each dsp output port carried at derive time
/// ([`shapes`][ResolvedPart::shapes]).
pub fn describe_parts(parts: &[ResolvedPart]) -> String {
    let mut s = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            s.push('\n');
        }
        describe_part(&mut s, i, part);
    }
    s
}

/// Render one part into `s`.
fn describe_part(s: &mut String, i: usize, part: &ResolvedPart) {
    let _ = writeln!(s, "part {i}: {}", part.def.name);
    let _ = writeln!(s, "  key {:016x}  sig {:016x}", part.key, part.sig);

    if !part.def.params.is_empty() {
        let _ = writeln!(s, "  params:");
        for (p, param) in part.def.params.iter().enumerate() {
            let bound = part
                .params
                .iter()
                .find(|b| b.index == p)
                .map(|b| format!("  <- {:?}", b.node_path));
            let fade = part.gains.iter().any(|g| g.index == p);
            let trig = param.is_trig.then_some("  trig");
            let lag = param.lag.map(|l| format!("  lag {l}"));
            let _ = writeln!(
                s,
                "    p{p} {} = {} {}{}{}{}",
                param.name,
                param.default,
                rate_token(param.rate),
                trig.unwrap_or(""),
                lag.as_deref().unwrap_or(""),
                bound
                    .as_deref()
                    .unwrap_or(if fade { "  (driver fade)" } else { "" }),
            );
        }
    }

    let _ = writeln!(s, "  units:");
    for (u, unit) in part.def.units.iter().enumerate() {
        let _ = writeln!(s, "    u{u} {}", unit_str(unit));
    }

    for m in &part.monitors {
        let _ = writeln!(
            s,
            "  monitor {:?}: {}ch ring {}",
            m.node_path, m.channels, m.size,
        );
    }
    for b in &part.bus_writes {
        let _ = writeln!(s, "  writes {}", bus_str(b));
    }
    for b in &part.bus_reads {
        let _ = writeln!(s, "  reads {}", bus_str(b));
    }

    if !part.shapes.is_empty() {
        let _ = writeln!(s, "  ports:");
        for ((path, port), shape) in &part.shapes {
            let _ = writeln!(
                s,
                "    {path:?}.{port}: {}ch {}",
                shape.width,
                rate_token(shape.rate),
            );
        }
    }
}

/// One unit as `Name rate (inputs) -> n_outs`, decoding an operator unit's
/// selector (e.g. `BinaryOpUGen[mul]`).
fn unit_str(unit: &UnitSpec) -> String {
    let op = match (unit.name.as_str(), unit.special_index) {
        // An index-0 `BinaryOpUGen` is the implicit `add` (derivation's
        // input summing) - common enough to display bare.
        ("BinaryOpUGen", 0) => String::new(),
        (name @ ("BinaryOpUGen" | "UnaryOpUGen"), i) => format!("[{}]", op_name(name, i)),
        (_, 0) => String::new(),
        (_, i) => format!("[{i}]"),
    };
    let inputs = unit
        .inputs
        .iter()
        .map(input_str)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{}{op} {} ({inputs}) -> {}",
        unit.name,
        rate_token(unit.rate),
        unit.num_outputs,
    )
}

/// One input wire, compactly: a bare literal, `p2` (param) or `u3.0`
/// (unit output).
fn input_str(input: &InputRef) -> String {
    match input {
        InputRef::Constant(c) => format!("{c}"),
        InputRef::Param(p) => format!("p{p}"),
        InputRef::Unit { unit, output } => format!("u{unit}.{output}"),
    }
}

/// One bus binding as `<key> Nch (u<unit>, p<param>)`.
fn bus_str(b: &ResolvedBus) -> String {
    format!(
        "{} {}ch (u{}, p{})",
        bus_key_str(&b.key),
        b.channels,
        b.unit,
        b.param,
    )
}

/// A bus key, compactly (absolute paths).
fn bus_key_str(key: &BusKey) -> String {
    match key {
        BusKey::Bus(path) => format!("~bus {path:?}"),
        BusKey::Src { path, output } => format!("src {path:?}.{output}"),
        BusKey::InstOut {
            path,
            outlet,
            summand,
        } => format!("inst {path:?} outlet {outlet} summand {summand}"),
        BusKey::IfaceIn { inlet, summand } => format!("inlet {inlet} summand {summand}"),
    }
}

/// The display token of a [`Rate`]: `ar`/`kr`/`ir`/`dr`.
pub(crate) fn rate_token(rate: Rate) -> &'static str {
    match rate {
        Rate::Audio => "ar",
        Rate::Control => "kr",
        Rate::Scalar => "ir",
        Rate::Demand => "dr",
    }
}

/// An operator-selector unit's operator name, from the descriptor table's
/// operator rows (the keyword sans `~`), falling back to `op{i}` for an
/// unmapped index.
fn op_name(unit: &str, index: i16) -> String {
    crate::units::UNITS
        .iter()
        .find(|d| {
            d.special
                .is_some_and(|s| s.unit == unit && s.index == index)
        })
        .map(|d| d.keyword[1..].to_string())
        .unwrap_or_else(|| format!("op{index}"))
}
