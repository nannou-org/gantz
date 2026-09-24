//! `.gantz` keyword sugar for [`Pplot`].
//!
//! A default pplot is a bare `pplot`. A pplot whose settings differ only in
//! span, resolution, value layout or body size is written with keywords, for
//! example `(pplot #:end 2 #:fit 2 #:expand #:width 240)`. Any other setting
//! falls back to the generic node form, so nothing is lost.

use super::{Pplot, Res, ValueLayout};
use gantz_egui::node::{F32, PlotLook};
use gantz_format::{Datum, FormatError, SugarArgs, from_datum, to_datum};
use gantz_nodetag::NodeTag;
use num_rational::Ratio;

/// The sugar keyword.
pub(crate) const KEYWORD: &str = "pplot";

/// The node's wire tag.
pub(crate) fn tag() -> &'static str {
    Pplot::TAG
}

/// Read a `(pplot <keyword>...)` form.
pub(crate) fn read_spec(args: SugarArgs<'_>) -> Result<Datum, FormatError> {
    let mut p = Pplot::default();
    if let Some(start) = ratio_keyword(&args, "start")? {
        p.start = start;
    }
    if let Some(end) = ratio_keyword(&args, "end")? {
        p.end = end;
    }
    if let Some(n) = args.keyword_int("fixed")? {
        p.res = Res::Fixed(n.clamp(1, i64::from(Res::MAX)) as u16);
    }
    if let Some(pts) = args.keyword_f64("fit")? {
        p.res = Res::Fit(F32(pts as f32));
    }
    if args.has_flag("expand") {
        p.layout = ValueLayout::Expand;
    }
    if let Some(w) = args.keyword_int("width")? {
        p.look.width = w.clamp(0, i64::from(u16::MAX)) as u16;
    }
    if let Some(h) = args.keyword_int("height")? {
        p.look.height = h.clamp(0, i64::from(u16::MAX)) as u16;
    }
    tagged(&p).ok_or_else(|| FormatError::malformed("pplot does not encode"))
}

/// The default pplot as a node datum.
pub(crate) fn read_bare() -> Option<Datum> {
    tagged(&Pplot::default())
}

/// Write a pplot as a bare `pplot` or a keyword form. `None` when a setting
/// has no keyword, so the generic node form applies.
pub(crate) fn write_spec(node: &Datum) -> Option<String> {
    let p: Pplot = from_datum(node.clone()).ok()?;
    let d = Pplot::default();

    // The pplot as the keywords can express it. Unless it encodes the same as
    // the pplot itself, some setting would be lost.
    let expressed = Pplot {
        start: p.start,
        end: p.end,
        res: p.res,
        layout: p.layout,
        key_colors: vec![],
        look: PlotLook {
            width: p.look.width,
            height: p.look.height,
            ..PlotLook::default()
        },
    };
    if to_datum(&expressed).ok()? != to_datum(&p).ok()? {
        return None;
    }

    let mut kws = vec![];
    if p.start != d.start {
        kws.push(format!("#:start {}", p.start));
    }
    if p.end != d.end {
        kws.push(format!("#:end {}", p.end));
    }
    match p.res {
        Res::Fixed(Res::DEFAULT_FIXED) => (),
        Res::Fixed(n) => kws.push(format!("#:fixed {n}")),
        Res::Fit(pts) => kws.push(format!("#:fit {}", pts.get())),
    }
    if p.layout == ValueLayout::Expand {
        kws.push("#:expand".to_string());
    }
    if p.look.width != d.look.width {
        kws.push(format!("#:width {}", p.look.width));
    }
    if p.look.height != d.look.height {
        kws.push(format!("#:height {}", p.look.height));
    }
    Some(match kws.is_empty() {
        true => KEYWORD.to_string(),
        false => format!("({KEYWORD} {})", kws.join(" ")),
    })
}

/// A pplot as a node datum, tagged with its wire tag.
fn tagged(p: &Pplot) -> Option<Datum> {
    match to_datum(p).ok()? {
        Datum::Map(fields) => Some(Datum::tagged(Pplot::TAG, fields)),
        _ => None,
    }
}

/// Read an exact rational keyword value, for example `2` or `1/3`.
fn ratio_keyword(args: &SugarArgs<'_>, key: &str) -> Result<Option<Ratio<i64>>, FormatError> {
    args.keyword_verbatim(key)?
        .map(|src| {
            crate::mini::parse_number(src)
                .ok_or_else(|| FormatError::malformed(format!("#:{key} requires a rational")))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_format::sexpr;

    fn read(text: &str) -> Datum {
        let exprs = sexpr::read(text).expect("read");
        let args = sexpr::list_args(&exprs[0]).expect("list");
        read_spec(SugarArgs::new(&args[1..], text)).expect("read_spec")
    }

    // A default pplot is bare. Keyword forms round-trip, including exact
    // rationals.
    #[test]
    fn keywords_round_trip() {
        let bare = read_bare().expect("bare");
        assert_eq!(write_spec(&bare).as_deref(), Some("pplot"));
        let text = "(pplot #:start 1/3 #:end 2 #:fit 2.5 #:expand #:width 240 #:height 60)";
        assert_eq!(write_spec(&read(text)).as_deref(), Some(text));
        let text = "(pplot #:fixed 64)";
        assert_eq!(write_spec(&read(text)).as_deref(), Some(text));
    }

    // Out-of-range integers clamp to the nearest valid setting.
    #[test]
    fn out_of_range_keywords_clamp() {
        let p: Pplot = from_datum(read("(pplot #:fixed -5 #:width -1 #:height 99999)")).unwrap();
        assert!(matches!(p.res, Res::Fixed(1)));
        assert_eq!((p.look.width, p.look.height), (0, u16::MAX));
        let p: Pplot = from_datum(read("(pplot #:fixed 99999)")).unwrap();
        assert!(matches!(p.res, Res::Fixed(Res::MAX)));
    }

    // A setting without a keyword falls back to the generic form.
    #[test]
    fn unexpressed_settings_fall_back() {
        let mut p = Pplot::default();
        p.look.show_grid = true;
        assert_eq!(write_spec(&tagged(&p).unwrap()), None);
        let mut p = Pplot::default();
        p.key_colors.push(super::super::KeyColor {
            key: "s".into(),
            color: [1, 2, 3, 255],
        });
        assert_eq!(write_spec(&tagged(&p).unwrap()), None);
    }
}
