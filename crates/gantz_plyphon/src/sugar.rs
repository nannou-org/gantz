//! `.gantz` keyword sugar for the DSP node set.
//!
//! [`PlyphonSugar`] provides the keywords for this crate's nodes. The bespoke
//! nodes read as bare `~out`, `~scopeout`, `~pack`, `~sum`, `~unpack`,
//! `~bus`, `~buffer` and `~sample`. The forms `(~out #:gain-lag s)`,
//! `(~scopeout #:size n)`, `(~pack #:count n)`, `(~sum #:count n)`,
//! `(~unpack #:count n)` and `(~buffer #:frames n #:channels c #:wavetable)`
//! carry the structural smoothing lag, ring length, socket count or buffer
//! shape. A `~sample` with an asset writes as a generic node, which keeps its
//! address. A bare `~envgen` is an ADSR. The form
//! `(~envgen #:init l #:segs ((level time [shape])...) #:release k #:rate kr
//! #:width w #:height h #:grid #:axes)` carries any other envelope or look.
//! A segment shape is a
//! name such as `exp`, or a number for a curve.
//! Every [`crate::units`] descriptor-table keyword reads and writes the same
//! way.
//! A bare form is `~sinosc` or `~lpf`. A full form such as
//! `(~combc #:delay-lag s #:maxdelay v #:rate kr)` carries the structural
//! per-param lags, init-only values and ugen rate. A fixed-rate row such as
//! `~a2k` accepts only its own rate and never writes one. Param values live
//! in VM state, not the node weight, so they are not serialized and never
//! appear here. Compose it with [`gantz_format::CoreSugar`] and the other
//! crates' sugars via [`gantz_format::Sugars`].

use gantz_format::{Datum, FormatError, Sugar, SugarArgs, from_datum, node_datum, to_datum};
use gantz_nodetag::NodeTag;

use crate::dsp::NodeRate;
use crate::envelope::{Envelope, Segment, Shape};
use crate::node::Envgen;
use crate::units::UnitRate;

/// Keyword sugar for the plyphon DSP nodes. It covers the bespoke
/// [`Out`](crate::Out), [`ScopeOut`](crate::ScopeOut), [`Pack`](crate::Pack),
/// [`Sum`](crate::Sum), [`Unpack`](crate::Unpack), [`Bus`](crate::Bus),
/// [`Buffer`](crate::Buffer), [`Sample`](crate::Sample) and
/// [`Envgen`] nodes plus every [`UnitNode`](crate::UnitNode)
/// descriptor-table keyword.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlyphonSugar;

/// Sugar keyword to typetag tag for the bespoke nodes. The descriptor-table
/// keywords all map to the `"Unit"` tag and resolve through
/// [`crate::units::unit_desc_by_keyword`].
const KEYWORD_TAG: &[(&str, &str)] = &[
    ("~out", "Out"),
    ("~scopeout", "ScopeOut"),
    ("~pack", "Pack"),
    ("~sum", "Sum"),
    ("~unpack", "Unpack"),
    ("~bus", "Bus"),
    ("~buffer", "Buffer"),
    ("~sample", "Sample"),
    ("~envgen", "Envgen"),
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
            "~out" => lag_spec("Out", "gain_lag", "gain-lag", args)?,
            "~scopeout" => size_spec(args)?,
            "~pack" => count_spec("Pack", args)?,
            "~sum" => count_spec("Sum", args)?,
            "~unpack" => count_spec("Unpack", args)?,
            "~bus" => node_datum("Bus", vec![]),
            "~buffer" => buffer_spec(args)?,
            "~sample" => node_datum("Sample", vec![]),
            "~envgen" => envgen_spec(args)?,
            other => match crate::units::unit_desc_by_keyword(other) {
                Some(desc) => unit_spec(desc, args)?,
                None => return Ok(None),
            },
        };
        Ok(Some(datum))
    }

    fn read_bare(&self, keyword: &str) -> Option<Datum> {
        // An envelope has no empty form, so the bare keyword is the default.
        if keyword == "~envgen" {
            return envgen_datum(&Envgen::default()).ok();
        }
        tag_for_keyword(keyword)
            .map(|tag| node_datum(tag, vec![]))
            .or_else(|| {
                crate::units::unit_desc_by_keyword(keyword)
                    .map(|desc| node_datum("Unit", vec![("unit", Datum::Str(desc.unit.into()))]))
            })
    }

    fn write_spec(&self, tag: &str, node: &Datum) -> Option<String> {
        match tag {
            "Out" => Some(write_lag(
                "~out",
                "gain_lag",
                "gain-lag",
                crate::Out::DEFAULT_GAIN_LAG,
                node,
            )),
            "ScopeOut" => Some(write_size(node)),
            "Pack" => Some(write_count("~pack", crate::Pack::DEFAULT_COUNT, node)),
            "Sum" => Some(write_count("~sum", crate::Sum::DEFAULT_COUNT, node)),
            "Unpack" => Some(write_count("~unpack", crate::Unpack::DEFAULT_COUNT, node)),
            "Buffer" => Some(write_buffer(node)),
            // An assigned sample falls through to the generic form, which
            // keeps its asset address.
            "Sample" => node.get("asset").is_none().then(|| "~sample".to_string()),
            "Envgen" => write_envgen(node),
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
            // One tag, many keywords. The stem comes from the `unit` field, so
            // `~lpf0`, not `~unit0`.
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
/// [#:rate ar|kr])` form into a `Unit` node datum. Each map or field is
/// carried only when its keyword is present, so a bare form stays bare.
/// Fields are pushed in `UnitNode`'s serde order, that is unit, rate, lags,
/// init. A fixed-rate row rejects any other rate and drops its own, so the
/// datum stays canonical.
fn unit_spec(desc: &'static crate::UnitDesc, args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut fields = vec![("unit", Datum::Str(desc.unit.into()))];
    push_rate(&mut fields, &args)?;
    if let UnitRate::Fixed(fixed) = desc.rate {
        if let Some(ix) = fields.iter().position(|(k, _)| *k == "rate") {
            let (_, rate) = fields.remove(ix);
            if rate.as_str() != Some(fixed.token()) {
                return Err(FormatError::malformed(format!(
                    "{} runs at `{}` only",
                    desc.keyword,
                    fixed.token()
                )));
            }
        }
    }
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

/// Write a `Unit` node datum. The bare keyword when everything is defaulted,
/// else `(<kw> [#:<param>-lag s]... [#:<init> v]... [#:rate kr])`. Stored
/// values are `f32`s widened to `f64`. Comparing and formatting them back as
/// `f32` keeps the form exact and tidy, as in [`write_lag`]. `None` means no
/// such unit in the table, which falls back to the generic
/// `(node "Unit" ...)` form.
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
    if desc.rate == UnitRate::Any {
        parts.extend(rate_part(node));
    }
    Some(write_form(desc.keyword, parts))
}

/// Read a `(<head> [#:<keyword> s] [#:rate ar|kr])` form into a node datum
/// tagged `tag`. The `field` lag and the ugen rate are carried only when
/// their keyword is present, so a bare form stays bare.
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

/// Write a node carrying a smoothing `field` lag and possibly a ugen rate.
/// The bare keyword `kw` when everything is at its default, else
/// `(<kw> [#:<keyword> <lag>] [#:rate kr])`. The stored lag is an `f32`
/// widened to `f64`. Comparing and formatting it back as `f32` keeps the form
/// exact and tidy, for example `0.01` rather than `0.00999999977648258`.
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

/// Read a `(~scopeout [#:size n])` form into a `ScopeOut` node datum,
/// carrying the ring `size` only when the keyword is present, so a bare form
/// stays bare.
fn size_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut fields = Vec::new();
    if let Some(size) = args.keyword_int("size")? {
        fields.push(("size", Datum::U64(size.max(1) as u64)));
    }
    Ok(node_datum("ScopeOut", fields))
}

/// Write a `ScopeOut`. The bare `~scopeout` when the ring `size` is at its
/// default, else `(~scopeout #:size n)`.
fn write_size(node: &Datum) -> String {
    match node.get("size").and_then(Datum::as_i64) {
        Some(size) if size != crate::ScopeOut::DEFAULT_SIZE as i64 => {
            format!("(~scopeout #:size {size})")
        }
        _ => "~scopeout".to_string(),
    }
}

/// Read a `(<head> [#:count n])` form into a node datum tagged `tag`,
/// carrying the socket `count` only when the keyword is present, so a bare
/// form stays bare.
fn count_spec(tag: &str, args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut fields = Vec::new();
    if let Some(count) = args.keyword_int("count")? {
        fields.push(("count", Datum::U64(count.max(1) as u64)));
    }
    Ok(node_datum(tag, fields))
}

/// Read a `(~buffer [#:frames n] [#:channels c] [#:wavetable])` form into a `Buffer` node
/// datum. Each field is carried only when its keyword is present, so a bare
/// form stays bare.
fn buffer_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut fields = Vec::new();
    if let Some(frames) = args.keyword_int("frames")? {
        let frames = (frames.max(1) as usize).min(crate::Buffer::MAX_FRAMES);
        fields.push(("frames", Datum::U64(frames as u64)));
    }
    if let Some(channels) = args.keyword_int("channels")? {
        let channels = (channels.max(1) as usize).min(crate::Buffer::MAX_CHANNELS);
        fields.push(("channels", Datum::U64(channels as u64)));
    }
    if args.has_flag("wavetable") {
        fields.push(("wavetable", Datum::Bool(true)));
    }
    Ok(node_datum("Buffer", fields))
}

/// Write a `Buffer`. The bare `~buffer` when its shape is the default, else
/// `(~buffer [#:frames n] [#:channels c] [#:wavetable])`.
fn write_buffer(node: &Datum) -> String {
    let mut parts = Vec::new();
    if let Some(frames) = node.get("frames").and_then(Datum::as_i64) {
        if frames != crate::Buffer::DEFAULT_FRAMES as i64 {
            parts.push(format!("#:frames {frames}"));
        }
    }
    if let Some(channels) = node.get("channels").and_then(Datum::as_i64) {
        if channels != crate::Buffer::DEFAULT_CHANNELS as i64 {
            parts.push(format!("#:channels {channels}"));
        }
    }
    if node.get("wavetable").and_then(Datum::as_bool) == Some(true) {
        parts.push("#:wavetable".to_string());
    }
    write_form("~buffer", parts)
}

/// Read a `(~envgen [#:init l] [#:segs (seg...)] [#:release k] [#:rate kr]
/// [#:width w] [#:height h] [#:grid] [#:axes])` form into an `Envgen` node
/// datum.
///
/// A segment is `(level time [shape])`. The shape is a name such as `lin` or
/// `exp`, see [`Shape::name`], or a number for a [`Shape::Curve`] segment
/// with that curve. It is `lin` when absent. Without `#:segs` the envelope is
/// the default ADSR. With `#:segs` there is no release point unless
/// `#:release` gives one.
fn envgen_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let default = Envelope::default();
    let init = args.keyword_f64("init")?.map_or(default.init, |l| l as f32);
    let (segments, release) = match args.keyword_list("segs")? {
        Some(segs) => {
            let segments = (0..segs.count())
                .map(|i| read_segment(&segs, i))
                .collect::<Result<Vec<_>, _>>()?;
            (segments, None)
        }
        None => (default.segments, default.release),
    };
    let release = match args.keyword_int("release")? {
        Some(k) => Some(k.max(0) as usize),
        None => release,
    };
    let rate = match args.keyword_symbol("rate")?.as_deref() {
        None | Some("ar") => NodeRate::Audio,
        Some("kr") => NodeRate::Control,
        Some(other) => {
            return Err(FormatError::malformed(format!(
                "#:rate must be `ar` or `kr`, got `{other}`"
            )));
        }
    };
    let mut node = Envgen::new(Envelope {
        init,
        segments,
        release,
    })
    .with_rate(rate);
    let [mut width, mut height] = node.size();
    if let Some(w) = args.keyword_int("width")? {
        width = w.clamp(0, i64::from(u16::MAX)) as u16;
    }
    if let Some(h) = args.keyword_int("height")? {
        height = h.clamp(0, i64::from(u16::MAX)) as u16;
    }
    node.set_size([width, height]);
    node.set_grid(args.has_flag("grid"));
    node.set_axes(args.has_flag("axes"));
    envgen_datum(&node)
}

/// Read segment `i` of a `#:segs` list, `(level time [shape])`.
fn read_segment(segs: &SugarArgs<'_>, i: usize) -> Result<Segment, FormatError> {
    let malformed = || segs.malformed_at(i, "a segment is `(level time [shape])`");
    let seg = segs.list_at(i).ok_or_else(malformed)?;
    let level = seg.f64_at(0)?.ok_or_else(malformed)? as f32;
    let time = seg.f64_at(1)?.ok_or_else(malformed)? as f32;
    if seg.count() < 3 {
        return Ok(Segment::new(level, time, Shape::Lin));
    }
    match seg.symbol_at(2) {
        Some(name) => Shape::from_name(&name)
            .map(|shape| Segment::new(level, time, shape))
            .ok_or_else(|| seg.malformed_at(2, format!("unknown segment shape `{name}`"))),
        None => {
            let curve = seg.f64_at(2)?.ok_or_else(malformed)? as f32;
            Ok(Segment::curved(level, time, curve))
        }
    }
}

/// Write an `Envgen`. The bare `~envgen` for the default node, else the
/// keyword form with each field that differs from the default. Every field
/// has a keyword, so the form never loses data.
fn write_envgen(node: &Datum) -> Option<String> {
    let node: Envgen = from_datum(node.clone()).ok()?;
    let env = node.envelope();
    let default = Envgen::default();
    let default_env = default.envelope();
    let mut parts = Vec::new();
    if env.init != default_env.init {
        parts.push(format!("#:init {}", env.init));
    }
    if (&env.segments, env.release) != (&default_env.segments, default_env.release) {
        let segs: Vec<String> = env.segments.iter().map(write_segment).collect();
        parts.push(format!("#:segs ({})", segs.join(" ")));
        if let Some(k) = env.release {
            parts.push(format!("#:release {k}"));
        }
    }
    if node.rate() != default.rate() {
        parts.push(format!("#:rate {}", node.rate().token()));
    }
    let ([width, height], [dw, dh]) = (node.size(), default.size());
    if width != dw {
        parts.push(format!("#:width {width}"));
    }
    if height != dh {
        parts.push(format!("#:height {height}"));
    }
    if node.grid() {
        parts.push("#:grid".to_string());
    }
    if node.axes() {
        parts.push("#:axes".to_string());
    }
    Some(write_form("~envgen", parts))
}

/// Write one segment, `(level time [shape])`.
fn write_segment(seg: &Segment) -> String {
    match seg.shape {
        Shape::Lin => format!("({} {})", seg.level, seg.time),
        Shape::Curve => format!("({} {} {})", seg.level, seg.time, seg.curve),
        shape => format!("({} {} {})", seg.level, seg.time, shape.name()),
    }
}

/// An `Envgen` as a node datum, tagged with its wire tag.
fn envgen_datum(node: &Envgen) -> Result<Datum, FormatError> {
    match to_datum(node) {
        Ok(Datum::Map(fields)) => Ok(Datum::tagged(Envgen::TAG, fields)),
        _ => Err(FormatError::malformed("`~envgen` does not encode")),
    }
}

/// Write a count node. The bare keyword `kw` when the socket `count` is at
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
        // A default `~sinosc` stays bare, read as a bare keyword or an empty
        // spec. A table keyword reads to a `Unit` datum carrying the unit name.
        let bare = s.read_bare("~sinosc").expect("bare");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("Unit"));
        assert_eq!(bare.get("unit").and_then(Datum::as_str), Some("SinOsc"));
        assert_eq!(s.write_spec("Unit", &bare).as_deref(), Some("~sinosc"));
        let empty = read_spec("(~sinosc)").expect("empty");
        assert_eq!(s.write_spec("Unit", &empty).as_deref(), Some("~sinosc"));
        // A non-default freq lag round-trips as a `lags` map entry.
        let lagged = read_spec("(~sinosc #:freq-lag 0.5)").expect("lagged");
        assert_eq!(
            lagged
                .get("lags")
                .and_then(|l| l.get("freq"))
                .and_then(Datum::as_f64),
            Some(0.5),
        );
        assert_eq!(
            s.write_spec("Unit", &lagged).as_deref(),
            Some("(~sinosc #:freq-lag 0.5)"),
        );
    }

    #[test]
    fn out_round_trips() {
        let s = PlyphonSugar;
        // A default `~out` stays bare, including a Datum carrying the default
        // lag. The f32 round-trips through f64 without tripping the default
        // check.
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
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("Unit"));
        assert_eq!(bare.get("unit").and_then(Datum::as_str), Some("Lag"));
        assert_eq!(s.write_spec("Unit", &bare).as_deref(), Some("~lag"));
        let spec = read_spec("(~lag)").expect("spec");
        assert_eq!(spec.get("unit").and_then(Datum::as_str), Some("Lag"));
        assert_eq!(s.write_spec("Unit", &spec).as_deref(), Some("~lag"));
    }

    #[test]
    fn tap_round_trips() {
        let s = PlyphonSugar;
        // A default `~scopeout` stays bare, read as a bare keyword or an empty
        // spec.
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
        // For `~sinosc`, bare stays bare. `ar` is the default and an explicit
        // `ar` writes bare. `kr` round-trips, alone or combined with a lag.
        let bare = read_spec("(~sinosc #:rate ar)").expect("ar");
        assert_eq!(s.write_spec("Unit", &bare).as_deref(), Some("~sinosc"));
        let kr = read_spec("(~sinosc #:rate kr)").expect("kr");
        assert_eq!(kr.get("rate").and_then(Datum::as_str), Some("kr"));
        assert_eq!(
            s.write_spec("Unit", &kr).as_deref(),
            Some("(~sinosc #:rate kr)"),
        );
        let both = read_spec("(~sinosc #:freq-lag 0.5 #:rate kr)").expect("both");
        assert_eq!(
            s.write_spec("Unit", &both).as_deref(),
            Some("(~sinosc #:freq-lag 0.5 #:rate kr)"),
        );
        // `~lag` takes the same form.
        let lag_kr = read_spec("(~lag #:rate kr)").expect("lag kr");
        assert_eq!(
            s.write_spec("Unit", &lag_kr).as_deref(),
            Some("(~lag #:rate kr)"),
        );
        assert_eq!(
            s.write_spec("Unit", &s.read_bare("~lag").expect("bare"))
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
    fn fixed_rate_rows_write_bare() {
        let s = PlyphonSugar;
        // A fixed-rate row drops its own rate. Any other rate is malformed.
        let kr = read_spec("(~a2k #:rate kr)").expect("kr");
        assert!(kr.get("rate").is_none());
        assert_eq!(s.write_spec("Unit", &kr).as_deref(), Some("~a2k"));
        let ar = read_spec("(~k2a #:rate ar)").expect("ar");
        assert_eq!(s.write_spec("Unit", &ar).as_deref(), Some("~k2a"));
        let exprs = sexpr::read("(~a2k #:rate ar)").expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        assert!(
            PlyphonSugar
                .read_spec("~a2k", SugarArgs::new(&args[1..], "(~a2k #:rate ar)"))
                .is_err(),
        );
        // A hand-built datum with the fixed rate also writes bare.
        let datum = node_datum(
            "Unit",
            vec![
                ("unit", Datum::Str("A2K".into())),
                ("rate", Datum::Str("kr".into())),
            ],
        );
        assert_eq!(s.write_spec("Unit", &datum).as_deref(), Some("~a2k"));
    }

    #[test]
    fn count_nodes_round_trip() {
        let s = PlyphonSugar;
        for (kw, tag, default) in [
            ("~pack", "Pack", crate::Pack::DEFAULT_COUNT),
            ("~sum", "Sum", crate::Sum::DEFAULT_COUNT),
            ("~unpack", "Unpack", crate::Unpack::DEFAULT_COUNT),
        ] {
            // A default count stays bare, read as a bare keyword or an empty
            // spec, including a Datum explicitly carrying the default.
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
        // Structural args round-trip, written in lags, init, rate order.
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
    fn operator_keywords_round_trip() {
        // Operator rows resolve through their pseudo `unit` identity like
        // any other table row. Bare keywords stay bare and the hybrid `b`
        // param's structural lag round-trips.
        let s = PlyphonSugar;
        let bare = s.read_bare("~tanh").expect("bare");
        assert_eq!(bare.get("type").and_then(Datum::as_str), Some("Unit"));
        assert_eq!(bare.get("unit").and_then(Datum::as_str), Some("TanH"));
        assert_eq!(s.write_spec("Unit", &bare).as_deref(), Some("~tanh"));
        let form = "(~mul #:b-lag 0.02 #:rate kr)";
        let mul = read_spec(form).expect("mul");
        assert_eq!(mul.get("unit").and_then(Datum::as_str), Some("Mul"));
        assert_eq!(
            mul.get("lags")
                .and_then(|l| l.get("b"))
                .and_then(Datum::as_f64),
            Some(0.02),
        );
        assert_eq!(s.write_spec("Unit", &mul).as_deref(), Some(form));
        assert_eq!(s.label_stem("Unit", &mul), Some("~mul"));
    }

    #[test]
    fn init_channels_round_trip() {
        let s = PlyphonSugar;
        let form = "(~grainbuf #:channels 4)";
        let grains = read_spec(form).expect("grains");
        assert_eq!(
            grains
                .get("init")
                .and_then(|i| i.get("channels"))
                .and_then(Datum::as_f64),
            Some(4.0),
        );
        assert_eq!(s.write_spec("Unit", &grains).as_deref(), Some(form));
    }

    #[test]
    fn buffer_and_sample_round_trip() {
        let s = PlyphonSugar;
        let bare = s.read_bare("~buffer").expect("bare");
        assert_eq!(s.write_spec("Buffer", &bare).as_deref(), Some("~buffer"));
        let form = "(~buffer #:frames 1024 #:channels 2)";
        let shaped = read_spec(form).expect("shaped");
        assert_eq!(shaped.get("frames").and_then(Datum::as_i64), Some(1024));
        assert_eq!(s.write_spec("Buffer", &shaped).as_deref(), Some(form));
        let form = "(~buffer #:frames 512 #:wavetable)";
        let table = read_spec(form).expect("wavetable");
        assert_eq!(table.get("wavetable").and_then(Datum::as_bool), Some(true));
        assert_eq!(s.write_spec("Buffer", &table).as_deref(), Some(form));
        // An unassigned sample is bare. An assigned one keeps the generic form.
        let sample = s.read_bare("~sample").expect("bare");
        assert_eq!(s.write_spec("Sample", &sample).as_deref(), Some("~sample"));
        let assigned = node_datum("Sample", vec![("asset", Datum::Str("ab".into()))]);
        assert_eq!(s.write_spec("Sample", &assigned), None);
    }

    /// The typed `Envgen` behind a node datum.
    fn envgen(d: &Datum) -> Envgen {
        from_datum(d.clone()).expect("an Envgen")
    }

    #[test]
    fn envgen_round_trips() {
        let s = PlyphonSugar;
        let bare = s.read_bare("~envgen").expect("bare");
        assert_eq!(envgen(&bare), Envgen::default());
        assert_eq!(s.write_spec("Envgen", &bare).as_deref(), Some("~envgen"));
        // Segments with the default shape, a curve and a named shape.
        let form = "(~envgen #:init 50 #:segs ((230 0.001) (50 0.15 -8) (0 1 exp)) \
                    #:release 2 #:rate kr #:width 240 #:grid #:axes)";
        let d = read_spec(form).expect("form");
        let node = envgen(&d);
        let env = node.envelope();
        assert_eq!(env.init, 50.0);
        assert_eq!(env.segments[0], Segment::new(230.0, 0.001, Shape::Lin));
        assert_eq!(env.segments[1], Segment::curved(50.0, 0.15, -8.0));
        assert_eq!(env.segments[2].shape, Shape::Exp);
        assert_eq!(env.release, Some(2));
        assert_eq!(node.rate(), NodeRate::Control);
        assert_eq!(node.size(), [240, Envgen::DEFAULT_HEIGHT]);
        assert!(node.grid() && node.axes());
        assert_eq!(s.write_spec("Envgen", &d).as_deref(), Some(form));
        // The default segments without their release point need `#:segs`.
        let mut no_release = Envelope::default();
        no_release.release = None;
        let d = envgen_datum(&Envgen::new(no_release.clone())).expect("datum");
        let written = s.write_spec("Envgen", &d).expect("form");
        assert_eq!(
            envgen(&read_spec(&written).expect("read")).envelope(),
            &no_release
        );
    }

    #[test]
    fn envgen_rejects_bad_segments() {
        let bad = |text: &str| {
            let exprs = sexpr::read(text).expect("read");
            let args = sexpr::list_args(&exprs[0]).expect("list");
            PlyphonSugar
                .read_spec("~envgen", SugarArgs::new(&args[1..], text))
                .is_err()
        };
        assert!(bad("(~envgen #:segs ((1)))"), "a segment needs a time");
        assert!(bad("(~envgen #:segs ((1 2 wobble)))"), "an unknown shape");
        assert!(bad("(~envgen #:segs 3)"), "segs must be a list");
        assert!(bad("(~envgen #:rate xr)"), "an unknown rate");
    }

    #[test]
    fn other_nodes_are_not_ours() {
        // A non-plyphon node falls through, so composition tries the next sugar.
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
