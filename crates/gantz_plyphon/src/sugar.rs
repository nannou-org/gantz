//! `.gantz` keyword sugar for the DSP node set.
//!
//! [`PlyphonSugar`] provides the human-friendly keywords for this crate's nodes:
//! bare `~sinosc`/`~out`/`~lag`/`~scopeout`/`~pack`/`~unpack`, with optional
//! `(~sinosc #:freq-lag s [#:rate ar|kr])`, `(~lag #:rate ar|kr)`,
//! `(~out #:gain-lag s)`, `(~scopeout #:size n)` and `(~pack #:count n)`/
//! `(~sum #:count n)`/
//! `(~unpack #:count n)` forms carrying the structural smoothing lag / ugen
//! rate / ring length / socket count. Every descriptor-table keyword (see
//! [`crate::units`]) reads/writes the same way: bare `~lpf`, or
//! `(~combc #:delay-lag s #:maxdelay v #:rate kr)` carrying the structural
//! per-param lags, init-only values and rate. Param *values* live in VM state
//! (not the node weight), so they are not serialized and never appear here.
//! Compose it with [`gantz_format::CoreSugar`] (and the other crates' sugars)
//! via [`gantz_format::Sugars`].

use gantz_format::{Datum, FormatError, Sugar, SugarArgs, node_datum};

/// Keyword sugar for the plyphon DSP nodes ([`SinOsc`](crate::SinOsc),
/// [`Out`](crate::Out), [`Lag`](crate::Lag), [`ScopeOut`](crate::ScopeOut),
/// [`Pack`](crate::Pack), [`Sum`](crate::Sum), [`Unpack`](crate::Unpack)).
#[derive(Clone, Copy, Debug, Default)]
pub struct PlyphonSugar;

/// Sugar keyword -> typetag tag.
const KEYWORD_TAG: &[(&str, &str)] = &[
    ("~sinosc", "SinOsc"),
    ("~out", "Out"),
    ("~lag", "Lag"),
    ("~scopeout", "ScopeOut"),
    ("~pack", "Pack"),
    ("~sum", "Sum"),
    ("~unpack", "Unpack"),
    ("~bus", "Bus"),
];

/// The typetag tag for a sugar keyword.
fn tag_for_keyword(kw: &str) -> Option<&'static str> {
    KEYWORD_TAG
        .iter()
        .find(|(k, _)| *k == kw)
        .map(|&(_, tag)| tag)
}

/// The sugar keyword for a typetag tag, if one exists.
fn keyword_for_tag(tag: &str) -> Option<&'static str> {
    KEYWORD_TAG
        .iter()
        .find(|(_, t)| *t == tag)
        .map(|&(kw, _)| kw)
}

impl Sugar for PlyphonSugar {
    fn read_spec(&self, head: &str, args: SugarArgs<'_>) -> Result<Option<Datum>, FormatError> {
        let datum = match head {
            "~sinosc" => lag_spec("SinOsc", "freq_lag", "freq-lag", args)?,
            "~out" => lag_spec("Out", "gain_lag", "gain-lag", args)?,
            "~lag" => rate_spec("Lag", args)?,
            "~scopeout" => size_spec(args)?,
            "~pack" => count_spec("Pack", args)?,
            "~sum" => count_spec("Sum", args)?,
            "~unpack" => count_spec("Unpack", args)?,
            "~bus" => node_datum("Bus", vec![]),
            other => match crate::units::unit_desc_by_keyword(other) {
                Some(desc) => unit_spec(desc, args)?,
                None => return Ok(None),
            },
        };
        Ok(Some(datum))
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        tag_for_keyword(keyword)
            .map(|tag| node_datum(tag, vec![]))
            .or_else(|| {
                crate::units::unit_desc_by_keyword(keyword)
                    .map(|desc| node_datum("Unit", vec![("unit", Datum::Str(desc.unit.into()))]))
            })
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        match tag {
            "SinOsc" => Some(write_lag(
                "~sinosc",
                "freq_lag",
                "freq-lag",
                crate::SinOsc::DEFAULT_FREQ_LAG,
                node,
            )),
            "Out" => Some(write_lag(
                "~out",
                "gain_lag",
                "gain-lag",
                crate::Out::DEFAULT_GAIN_LAG,
                node,
            )),
            "Lag" => Some(write_form("~lag", rate_part(node).into_iter().collect())),
            "ScopeOut" => Some(write_size(node)),
            "Pack" => Some(write_count("~pack", crate::Pack::DEFAULT_COUNT, node)),
            "Sum" => Some(write_count("~sum", crate::Sum::DEFAULT_COUNT, node)),
            "Unpack" => Some(write_count("~unpack", crate::Unpack::DEFAULT_COUNT, node)),
            // An unknown unit name falls through to the generic form.
            "Unit" => write_unit(node),
            other => keyword_for_tag(other).map(str::to_string),
        }
    }

    fn keyword_for_tag(&self, tag: &str) -> Option<&str> {
        keyword_for_tag(tag)
    }

    fn label_stem(&self, tag: &str, node: &Datum) -> Option<&str> {
        match tag {
            // One tag, many keywords: the stem comes from the `unit` field
            // (`~lpf0`, not `~unit0`).
            "Unit" => node
                .get("unit")
                .and_then(Datum::as_str)
                .and_then(crate::units::unit_desc)
                .map(|desc| desc.keyword),
            other => keyword_for_tag(other),
        }
    }
}

/// Read a table keyword's `(<kw> [#:<param>-lag s]... [#:<init> v]...
/// [#:rate ar|kr])` form into a `Unit` node datum, carrying each map/field
/// only when its keyword is present (so a bare form stays bare). Fields are
/// pushed in `UnitNode`'s serde order (unit, rate, lags, init).
fn unit_spec(desc: &'static crate::UnitDesc, args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut fields = vec![("unit", Datum::Str(desc.unit.into()))];
    push_rate(&mut fields, &args)?;
    let mut lags = Vec::new();
    for (name, _) in desc.hybrid_params() {
        if let Some(lag) = args.keyword_f64(&format!("{name}-lag"))? {
            lags.push((name.to_string(), Datum::F64(lag)));
        }
    }
    if !lags.is_empty() {
        fields.push(("lags", Datum::Map(lags)));
    }
    let mut init = Vec::new();
    for (name, _) in desc.init_params() {
        if let Some(value) = args.keyword_f64(name)? {
            init.push((name.to_string(), Datum::F64(value)));
        }
    }
    if !init.is_empty() {
        fields.push(("init", Datum::Map(init)));
    }
    Ok(node_datum("Unit", fields))
}

/// Write a `Unit` node datum: the bare keyword when everything is defaulted,
/// else `(<kw> [#:<param>-lag s]... [#:<init> v]... [#:rate kr])`. Stored
/// values are `f32`s widened to `f64`; comparing and formatting them back as
/// `f32` keeps the form exact and tidy (as [`write_lag`]). `None` (no such
/// unit in the table) falls back to the generic `(node "Unit" ...)` form.
fn write_unit(node: &Datum) -> Option<String> {
    let desc = node
        .get("unit")
        .and_then(Datum::as_str)
        .and_then(crate::units::unit_desc)?;
    let mut parts = Vec::new();
    if let Some(lags) = node.get("lags") {
        for (name, _) in desc.hybrid_params() {
            if let Some(lag) = lags.get(name).and_then(Datum::as_f64) {
                if lag as f32 != 0.0 {
                    parts.push(format!("#:{name}-lag {}", lag as f32));
                }
            }
        }
    }
    if let Some(init) = node.get("init") {
        for (name, default) in desc.init_params() {
            if let Some(value) = init.get(name).and_then(Datum::as_f64) {
                if value as f32 != default {
                    parts.push(format!("#:{name} {}", value as f32));
                }
            }
        }
    }
    parts.extend(rate_part(node));
    Some(write_form(desc.keyword, parts))
}

/// Read a `(<head> [#:<keyword> s] [#:rate ar|kr])` form into a node datum tagged
/// `tag`, carrying the `field` lag / the ugen rate only when the keyword is
/// present (so a bare form stays bare).
fn lag_spec(
    tag: &str,
    field: &str,
    keyword: &str,
    args: SugarArgs<'_>,
) -> Result<Datum, FormatError> {
    let mut fields = Vec::new();
    if let Some(lag) = args.keyword_f64(keyword)? {
        fields.push((field, Datum::F64(lag)));
    }
    push_rate(&mut fields, &args)?;
    Ok(node_datum(tag, fields))
}

/// Read a `(<head> [#:rate ar|kr])` form into a node datum tagged `tag`.
fn rate_spec(tag: &str, args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut fields = Vec::new();
    push_rate(&mut fields, &args)?;
    Ok(node_datum(tag, fields))
}

/// Read an optional `#:rate ar|kr` keyword into a node datum `rate` field.
fn push_rate<'a>(
    fields: &mut Vec<(&'a str, Datum)>,
    args: &SugarArgs<'_>,
) -> Result<(), FormatError> {
    if let Some(rate) = args.keyword_symbol("rate")? {
        match rate.as_str() {
            "ar" | "kr" => fields.push(("rate", Datum::Str(rate))),
            other => {
                return Err(FormatError::malformed(format!(
                    "#:rate must be `ar` or `kr`, got `{other}`"
                )));
            }
        }
    }
    Ok(())
}

/// Write a node carrying a smoothing `field` lag (and possibly a ugen rate): the
/// bare keyword `kw` when everything is at its default, else
/// `(<kw> [#:<keyword> <lag>] [#:rate kr])`. The stored lag is an `f32` widened
/// to `f64`; comparing and formatting it back as `f32` keeps the form exact and
/// tidy (e.g. `0.01`, not `0.00999999977648258`).
fn write_lag(kw: &str, field: &str, keyword: &str, default: f32, node: &Datum) -> String {
    let mut parts = Vec::new();
    if let Some(lag) = node.get(field).and_then(Datum::as_f64) {
        if lag as f32 != default {
            parts.push(format!("#:{keyword} {}", lag as f32));
        }
    }
    parts.extend(rate_part(node));
    write_form(kw, parts)
}

/// A `#:rate kr` part for a node datum carrying a non-default ugen rate.
fn rate_part(node: &Datum) -> Option<String> {
    match node.get("rate").and_then(Datum::as_str) {
        Some(rate) if rate != "ar" => Some(format!("#:rate {rate}")),
        _ => None,
    }
}

/// The bare keyword `kw` when there are no keyword `parts`, else `(<kw> <parts>)`.
fn write_form(kw: &str, parts: Vec<String>) -> String {
    match parts.is_empty() {
        true => kw.to_string(),
        false => format!("({kw} {})", parts.join(" ")),
    }
}

/// Read a `(~scopeout [#:size n])` form into a `ScopeOut` node datum, carrying the ring
/// `size` only when the keyword is present (so a bare form stays bare).
fn size_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut fields = Vec::new();
    if let Some(size) = args.keyword_int("size")? {
        fields.push(("size", Datum::U64(size.max(1) as u64)));
    }
    Ok(node_datum("ScopeOut", fields))
}

/// Write a `ScopeOut`: the bare `~scopeout` when the ring `size` is at its default, else
/// `(~scopeout #:size n)`.
fn write_size(node: &Datum) -> String {
    match node.get("size").and_then(Datum::as_i64) {
        Some(size) if size != crate::ScopeOut::DEFAULT_SIZE as i64 => {
            format!("(~scopeout #:size {size})")
        }
        _ => "~scopeout".to_string(),
    }
}

/// Read a `(<head> [#:count n])` form into a node datum tagged `tag`, carrying the
/// socket `count` only when the keyword is present (so a bare form stays bare).
fn count_spec(tag: &str, args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut fields = Vec::new();
    if let Some(count) = args.keyword_int("count")? {
        fields.push(("count", Datum::U64(count.max(1) as u64)));
    }
    Ok(node_datum(tag, fields))
}

/// Write a `Pack`/`Unpack`: the bare keyword `kw` when the socket `count` is at
/// `default`, else `(<kw> #:count n)`.
fn write_count(kw: &str, default: usize, node: &Datum) -> String {
    match node.get("count").and_then(Datum::as_i64) {
        Some(count) if count != default as i64 => format!("({kw} #:count {count})"),
        _ => kw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_format::sexpr;

    /// Read a single sugar form's text through `PlyphonSugar`, as the format does.
    fn read_spec(text: &str) -> Option<Datum> {
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        let head = sexpr::as_symbol(&args[0]).expect("head");
        PlyphonSugar
            .read_spec(&head, SugarArgs::new(&args[1..], text))
            .expect("read_spec")
    }

    #[test]
    fn sine_round_trips() {
        let s = PlyphonSugar;
        // A default `~sinosc` stays bare, read as a bare keyword or an empty spec.
        let bare = s.read_bare("~sinosc").expect("bare");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("SinOsc"));
        assert_eq!(s.write_spec("SinOsc", &bare).as_deref(), Some("~sinosc"));
        let empty = read_spec("(~sinosc)").expect("empty");
        assert_eq!(s.write_spec("SinOsc", &empty).as_deref(), Some("~sinosc"));
        // A non-default freq lag round-trips.
        let lagged = read_spec("(~sinosc #:freq-lag 0.5)").expect("lagged");
        assert_eq!(lagged.get("freq_lag").and_then(Datum::as_f64), Some(0.5));
        assert_eq!(
            s.write_spec("SinOsc", &lagged).as_deref(),
            Some("(~sinosc #:freq-lag 0.5)"),
        );
    }

    #[test]
    fn out_round_trips() {
        let s = PlyphonSugar;
        // A default `~out` stays bare - including a Datum carrying the default lag
        // (the f32 round-trips through f64 without tripping the default check).
        let bare = s.read_bare("~out").expect("bare");
        assert_eq!(s.write_spec("Out", &bare).as_deref(), Some("~out"));
        let defaulted = node_datum(
            "Out",
            vec![(
                "gain_lag",
                Datum::F64(f64::from(crate::Out::DEFAULT_GAIN_LAG)),
            )],
        );
        assert_eq!(s.write_spec("Out", &defaulted).as_deref(), Some("~out"));
        // A non-default gain lag round-trips, tidily.
        let lagged = read_spec("(~out #:gain-lag 0.02)").expect("lagged");
        assert_eq!(
            s.write_spec("Out", &lagged).as_deref(),
            Some("(~out #:gain-lag 0.02)"),
        );
    }

    #[test]
    fn lag_round_trips() {
        let s = PlyphonSugar;
        let bare = s.read_bare("~lag").expect("bare");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("Lag"));
        assert_eq!(s.write_spec("Lag", &bare).as_deref(), Some("~lag"));
        let spec = read_spec("(~lag)").expect("spec");
        assert_eq!(spec.get("type").and_then(Datum::as_str), Some("Lag"));
        assert_eq!(s.write_spec("Lag", &spec).as_deref(), Some("~lag"));
    }

    #[test]
    fn tap_round_trips() {
        let s = PlyphonSugar;
        // A default `~scopeout` stays bare (read as a bare keyword or an empty spec).
        let bare = s.read_bare("~scopeout").expect("bare");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("ScopeOut"));
        assert_eq!(
            s.write_spec("ScopeOut", &bare).as_deref(),
            Some("~scopeout")
        );
        let empty = read_spec("(~scopeout)").expect("empty");
        assert_eq!(
            s.write_spec("ScopeOut", &empty).as_deref(),
            Some("~scopeout")
        );
        // A `ScopeOut` carrying the default size still writes bare.
        let defaulted = node_datum(
            "ScopeOut",
            vec![("size", Datum::U64(crate::ScopeOut::DEFAULT_SIZE as u64))],
        );
        assert_eq!(
            s.write_spec("ScopeOut", &defaulted).as_deref(),
            Some("~scopeout")
        );
        // A non-default size round-trips.
        let sized = read_spec("(~scopeout #:size 512)").expect("sized");
        assert_eq!(sized.get("size").and_then(Datum::as_i64), Some(512));
        assert_eq!(
            s.write_spec("ScopeOut", &sized).as_deref(),
            Some("(~scopeout #:size 512)"),
        );
    }

    #[test]
    fn rate_round_trips() {
        let s = PlyphonSugar;
        // `~sinosc`: bare stays bare (ar is the default, and an explicit `ar`
        // writes bare); `kr` round-trips, alone or combined with a lag.
        let bare = read_spec("(~sinosc #:rate ar)").expect("ar");
        assert_eq!(s.write_spec("SinOsc", &bare).as_deref(), Some("~sinosc"));
        let kr = read_spec("(~sinosc #:rate kr)").expect("kr");
        assert_eq!(kr.get("rate").and_then(Datum::as_str), Some("kr"));
        assert_eq!(
            s.write_spec("SinOsc", &kr).as_deref(),
            Some("(~sinosc #:rate kr)"),
        );
        let both = read_spec("(~sinosc #:freq-lag 0.5 #:rate kr)").expect("both");
        assert_eq!(
            s.write_spec("SinOsc", &both).as_deref(),
            Some("(~sinosc #:freq-lag 0.5 #:rate kr)"),
        );
        // `~lag` gains the same form.
        let lag_kr = read_spec("(~lag #:rate kr)").expect("lag kr");
        assert_eq!(
            s.write_spec("Lag", &lag_kr).as_deref(),
            Some("(~lag #:rate kr)"),
        );
        assert_eq!(
            s.write_spec("Lag", &s.read_bare("~lag").expect("bare"))
                .as_deref(),
            Some("~lag"),
        );
        // Anything but ar/kr is malformed.
        let exprs = sexpr::read("(~sinosc #:rate dr)").expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        assert!(
            PlyphonSugar
                .read_spec("~sinosc", SugarArgs::new(&args[1..], "(~sinosc #:rate dr)"))
                .is_err(),
        );
    }

    #[test]
    fn count_nodes_round_trip() {
        let s = PlyphonSugar;
        for (kw, tag, default) in [
            ("~pack", "Pack", crate::Pack::DEFAULT_COUNT),
            ("~sum", "Sum", crate::Sum::DEFAULT_COUNT),
            ("~unpack", "Unpack", crate::Unpack::DEFAULT_COUNT),
        ] {
            // A default count stays bare (read as a bare keyword or an empty spec),
            // including a Datum explicitly carrying the default.
            let bare = s.read_bare(kw).expect("bare");
            assert_eq!(bare.get("type").and_then(Datum::as_str), Some(tag));
            assert_eq!(s.write_spec(tag, &bare).as_deref(), Some(kw));
            let empty = read_spec(&format!("({kw})")).expect("empty");
            assert_eq!(s.write_spec(tag, &empty).as_deref(), Some(kw));
            let defaulted = node_datum(tag, vec![("count", Datum::U64(default as u64))]);
            assert_eq!(s.write_spec(tag, &defaulted).as_deref(), Some(kw));
            // A non-default count round-trips.
            let form = format!("({kw} #:count 4)");
            let counted = read_spec(&form).expect("counted");
            assert_eq!(counted.get("count").and_then(Datum::as_i64), Some(4));
            assert_eq!(s.write_spec(tag, &counted).as_deref(), Some(form.as_str()));
        }
    }

    #[test]
    fn unit_keywords_round_trip() {
        let s = PlyphonSugar;
        // A bare table keyword reads to a `Unit` datum and writes back bare.
        let bare = s.read_bare("~lpf").expect("bare");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("Unit"));
        assert_eq!(bare.get("unit").and_then(Datum::as_str), Some("LPF"));
        assert_eq!(s.write_spec("Unit", &bare).as_deref(), Some("~lpf"));
        // Structural args round-trip (written in lags/init/rate order).
        let form = "(~combc #:delay-lag 0.02 #:maxdelay 0.5 #:rate kr)";
        let combc = read_spec(form).expect("combc");
        assert_eq!(combc.get("unit").and_then(Datum::as_str), Some("CombC"));
        assert_eq!(
            combc
                .get("lags")
                .and_then(|l| l.get("delay"))
                .and_then(Datum::as_f64),
            Some(0.02),
        );
        assert_eq!(
            combc
                .get("init")
                .and_then(|i| i.get("maxdelay"))
                .and_then(Datum::as_f64),
            Some(0.5),
        );
        assert_eq!(s.write_spec("Unit", &combc).as_deref(), Some(form));
        // Defaulted lag/init entries write back bare.
        let defaulted = node_datum(
            "Unit",
            vec![
                ("unit", Datum::Str("CombC".into())),
                (
                    "lags",
                    Datum::Map(vec![("delay".to_string(), Datum::F64(0.0))]),
                ),
                (
                    "init",
                    Datum::Map(vec![("maxdelay".to_string(), Datum::F64(0.2))]),
                ),
            ],
        );
        assert_eq!(s.write_spec("Unit", &defaulted).as_deref(), Some("~combc"));
        // An unknown unit name falls back to the generic form.
        let unknown = node_datum("Unit", vec![("unit", Datum::Str("NoSuchUnit".into()))]);
        assert_eq!(s.write_spec("Unit", &unknown), None);
    }

    #[test]
    fn unit_label_stems_come_from_the_unit_field() {
        let s = PlyphonSugar;
        let lpf = s.read_bare("~lpf").expect("bare");
        assert_eq!(s.label_stem("Unit", &lpf), Some("~lpf"));
        let pan = s.read_bare("~pan2").expect("bare");
        assert_eq!(s.label_stem("Unit", &pan), Some("~pan2"));
        // The default still applies to the bespoke tags.
        assert_eq!(
            s.label_stem("Out", &node_datum("Out", vec![])),
            Some("~out")
        );
    }

    #[test]
    fn other_nodes_are_not_ours() {
        // A non-plyphon node falls through (so composition tries the next sugar).
        let exprs = sexpr::read("(number 5)").expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        assert!(
            PlyphonSugar
                .read_spec("number", SugarArgs::new(&args[1..], "(number 5)"))
                .expect("ok")
                .is_none()
        );
        assert!(PlyphonSugar.read_bare("number").is_none());
        assert!(
            PlyphonSugar
                .write_spec("Number", &node_datum("Number", vec![]))
                .is_none()
        );
    }
}
