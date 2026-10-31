//! The figures that a `canvas` draws.
//!
//! A canvas binds a state value. The value is a list of panels, which stack
//! top to bottom. A panel holds figures in its own data space:
//!
//! ```text
//! panel := (panel attrs? figure ...)
//! figure := (seg attrs? x0 y0 x1 y1)
//!         | (dot attrs? x y)
//!         | (line attrs? (x y) ...)
//!         | (rect attrs? x0 y0 x1 y1)
//! ```
//!
//! Panel attributes: `x` fixes the x range as two numbers, else it fits the
//! figures. `y` fixes the y range the same way, and `y-min` or `y-max` fix
//! one edge of it. `pad` keeps that many points beyond each side of the x
//! range, and between the figures and each fitted edge of the y range. A
//! fixed y edge stays exact. `title` is text at the top left.
//!
//! Figure attributes: `color` (`"#rrggbb"` or `"#rrggbbaa"`, theme default
//! when absent) and `tip` (text shown on hover). A `rect` also takes `label`
//! (text drawn within it) and `frac` (when true, its `y` values are fractions
//! of the panel height from the bottom, so it keeps its place in any range).
//!
//! The decode is total. A malformed panel or figure is dropped with a
//! warning, and the rest still draws. A single panel form also decodes, as a
//! list of one.

use crate::elem::Rgba;
use crate::sexpr;
use gantz_ca::Datum;

/// The decoded figures of a canvas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Figures {
    /// The panels, top to bottom.
    pub panels: Vec<Panel>,
    /// What was dropped, and why.
    pub warnings: Vec<String>,
}

/// One plot area of a canvas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panel {
    /// The fixed x range, else it fits the figures.
    pub x: Option<[f64; 2]>,
    /// The fixed lower edge of the y range, else it fits the figures.
    pub y_min: Option<f64>,
    /// The fixed upper edge of the y range, else it fits the figures.
    pub y_max: Option<f64>,
    /// The points beyond each side of the x range, and between the figures
    /// and each fitted edge of the y range.
    pub pad: f32,
    /// Text at the top left.
    pub title: Option<String>,
    pub figures: Vec<Figure>,
}

/// One shape of a panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Figure {
    /// A line segment.
    Seg(Seg),
    /// A filled dot.
    Dot(Dot),
    /// A line through points.
    Line(Line),
    /// A filled, outlined rectangle.
    Rect(Rect),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seg {
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub color: Option<Rgba>,
    pub tip: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dot {
    pub at: [f64; 2],
    pub color: Option<Rgba>,
    pub tip: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub points: Vec<[f64; 2]>,
    pub color: Option<Rgba>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rect {
    pub min: [f64; 2],
    pub max: [f64; 2],
    /// Whether the `y` values are fractions of the panel height.
    pub frac: bool,
    pub color: Option<Rgba>,
    pub label: Option<String>,
    pub tip: Option<String>,
}

/// The most figures that a canvas draws. The rest are dropped with a warning.
pub const MAX_FIGURES: usize = 100_000;

/// The attributes of one form, by name.
struct Attrs(Vec<(String, Vec<Datum>)>);

impl Figure {
    /// The `y` values of the figure in data space, which a fitted range
    /// covers. A `frac` rect has none.
    pub fn ys(&self) -> Vec<f64> {
        match self {
            Figure::Seg(s) => vec![s.from[1], s.to[1]],
            Figure::Dot(d) => vec![d.at[1]],
            Figure::Line(l) => l.points.iter().map(|p| p[1]).collect(),
            Figure::Rect(r) if r.frac => vec![],
            Figure::Rect(r) => vec![r.min[1], r.max[1]],
        }
    }

    /// The `x` values of the figure, which a fitted range covers.
    pub fn xs(&self) -> Vec<f64> {
        match self {
            Figure::Seg(s) => vec![s.from[0], s.to[0]],
            Figure::Dot(d) => vec![d.at[0]],
            Figure::Line(l) => l.points.iter().map(|p| p[0]).collect(),
            Figure::Rect(r) => vec![r.min[0], r.max[0]],
        }
    }
}

impl Attrs {
    fn take(&mut self, name: &str) -> Option<Vec<Datum>> {
        let ix = self.0.iter().position(|(n, _)| n == name)?;
        Some(self.0.remove(ix).1)
    }

    fn text(&mut self, name: &str) -> Option<String> {
        match self.take(name)?.as_slice() {
            [v] => text(v),
            _ => None,
        }
    }

    fn color(&mut self, name: &str) -> Option<Rgba> {
        self.text(name).as_deref().and_then(Rgba::parse)
    }

    fn bool(&mut self, name: &str) -> bool {
        matches!(self.take(name).as_deref(), Some([Datum::Bool(true)]))
    }

    fn num(&mut self, name: &str) -> Option<f64> {
        match self.take(name)?.as_slice() {
            [v] => num(v),
            _ => None,
        }
    }

    fn range(&mut self, name: &str) -> Option<[f64; 2]> {
        match self.take(name)?.as_slice() {
            [a, b] => Some([num(a)?, num(b)?]),
            _ => None,
        }
    }
}

/// Decode the bound state of a canvas.
pub fn decode_figures(expr: &Datum) -> Figures {
    let mut out = Figures::default();
    let Datum::Seq(items) = expr else {
        out.warnings.push(format!(
            "expected a list of panels, found {}",
            sexpr::summary(expr)
        ));
        return out;
    };
    let panels: Vec<&Datum> = match items.first() {
        Some(head) if ident(head).as_deref() == Some("panel") => vec![expr],
        _ => items.iter().collect(),
    };
    let mut budget = MAX_FIGURES;
    for (ix, p) in panels.into_iter().enumerate() {
        match panel(p, &mut budget, &mut out.warnings) {
            Some(panel) => out.panels.push(panel),
            None => out.warnings.push(format!("panel {ix}: not a panel form")),
        }
    }
    out
}

/// Decode one panel form, or `None` when it is not one.
fn panel(expr: &Datum, budget: &mut usize, warnings: &mut Vec<String>) -> Option<Panel> {
    let (tag, mut attrs, args) = form(expr)?;
    if tag != "panel" {
        return None;
    }
    let y = attrs.range("y");
    let mut panel = Panel {
        x: attrs.range("x"),
        y_min: attrs.num("y-min").or(y.map(|y| y[0])),
        y_max: attrs.num("y-max").or(y.map(|y| y[1])),
        pad: attrs.num("pad").map_or(0.0, |p| p.max(0.0) as f32),
        title: attrs.text("title"),
        figures: Vec::with_capacity(args.len()),
    };
    for (ix, arg) in args.iter().enumerate() {
        if *budget == 0 {
            warnings.push(format!(
                "more than {MAX_FIGURES} figures, the rest are dropped"
            ));
            break;
        }
        match figure(arg) {
            Some(f) => {
                panel.figures.push(f);
                *budget -= 1;
            }
            None => warnings.push(format!("figure {ix}: {}", sexpr::summary(arg))),
        }
    }
    Some(panel)
}

/// Decode one figure form, or `None` when it is malformed.
fn figure(expr: &Datum) -> Option<Figure> {
    let (tag, mut attrs, args) = form(expr)?;
    let color = attrs.color("color");
    let tip = attrs.text("tip");
    let nums = || args.iter().map(num).collect::<Option<Vec<f64>>>();
    match tag.as_str() {
        "seg" => match nums()?.as_slice() {
            &[x0, y0, x1, y1] => Some(Figure::Seg(Seg {
                from: [x0, y0],
                to: [x1, y1],
                color,
                tip,
            })),
            _ => None,
        },
        "dot" => match nums()?.as_slice() {
            &[x, y] => Some(Figure::Dot(Dot {
                at: [x, y],
                color,
                tip,
            })),
            _ => None,
        },
        "line" => {
            let points = args
                .iter()
                .map(|p| match p {
                    Datum::Seq(xy) => match xy.as_slice() {
                        [x, y] => Some([num(x)?, num(y)?]),
                        _ => None,
                    },
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            Some(Figure::Line(Line { points, color }))
        }
        "rect" => match nums()?.as_slice() {
            &[x0, y0, x1, y1] => Some(Figure::Rect(Rect {
                min: [x0.min(x1), y0.min(y1)],
                max: [x0.max(x1), y0.max(y1)],
                frac: attrs.bool("frac"),
                color,
                label: attrs.text("label"),
                tip,
            })),
            _ => None,
        },
        _ => None,
    }
}

/// The tag, attributes and arguments of a form `(tag (@ ...)? arg ...)`.
fn form(expr: &Datum) -> Option<(String, Attrs, &[Datum])> {
    let Datum::Seq(items) = expr else {
        return None;
    };
    let (head, rest) = items.split_first()?;
    let tag = ident(head)?;
    let (attrs, args) = match rest.split_first() {
        Some((Datum::Seq(block), args))
            if block.first().and_then(ident).as_deref() == Some(crate::elem::ATTRS_MARKER) =>
        {
            (attr_entries(&block[1..]), args)
        }
        _ => (Attrs(vec![]), rest),
    };
    Some((tag, attrs, args))
}

/// The entries `(name value ...)` of an attribute block.
fn attr_entries(entries: &[Datum]) -> Attrs {
    let entries = entries
        .iter()
        .filter_map(|e| match e {
            Datum::Seq(kv) => {
                let (name, values) = kv.split_first()?;
                Some((ident(name)?, values.to_vec()))
            }
            _ => None,
        })
        .collect();
    Attrs(entries)
}

/// An identifier, or a string in its place.
fn ident(expr: &Datum) -> Option<String> {
    match expr {
        Datum::Sym(s) | Datum::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// Text, or an identifier in its place.
fn text(expr: &Datum) -> Option<String> {
    ident(expr)
}

/// A finite number.
fn num(expr: &Datum) -> Option<f64> {
    match expr {
        Datum::I64(i) => Some(*i as f64),
        Datum::F64(f) if f.is_finite() => Some(*f),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> Datum {
        Datum::Sym(s.into())
    }

    fn list(items: Vec<Datum>) -> Datum {
        Datum::Seq(items)
    }

    fn attrs(entries: Vec<(&str, Vec<Datum>)>) -> Datum {
        let mut block = vec![id("@")];
        for (name, values) in entries {
            let mut e = vec![id(name)];
            e.extend(values);
            block.push(list(e));
        }
        list(block)
    }

    #[test]
    fn panels_and_figures_decode() {
        let state = list(vec![list(vec![
            id("panel"),
            attrs(vec![
                ("x", vec![Datum::I64(0), Datum::F64(1.0)]),
                ("pad", vec![Datum::I64(4)]),
                ("title", vec![Datum::Str("kick".into())]),
            ]),
            list(vec![
                id("seg"),
                attrs(vec![
                    ("color", vec![Datum::Str("#ff0000".into())]),
                    ("tip", vec![Datum::Str("1".into())]),
                ]),
                Datum::F64(0.0),
                Datum::I64(1),
                Datum::F64(0.5),
                Datum::I64(1),
            ]),
            list(vec![id("dot"), Datum::F64(0.0), Datum::I64(1)]),
            list(vec![
                id("line"),
                list(vec![Datum::I64(0), Datum::I64(0)]),
                list(vec![Datum::F64(0.5), Datum::F64(2.0)]),
            ]),
            list(vec![
                id("rect"),
                attrs(vec![
                    ("frac", vec![Datum::Bool(true)]),
                    ("label", vec![Datum::Str("bd".into())]),
                ]),
                Datum::F64(0.5),
                Datum::F64(0.9),
                Datum::F64(0.25),
                Datum::F64(0.1),
            ]),
        ])]);
        let figs = decode_figures(&state);
        assert!(figs.warnings.is_empty(), "{:?}", figs.warnings);
        let panel = &figs.panels[0];
        assert_eq!(panel.x, Some([0.0, 1.0]));
        assert_eq!((panel.y_min, panel.y_max), (None, None));
        assert_eq!(panel.pad, 4.0);
        assert_eq!(panel.title.as_deref(), Some("kick"));
        assert_eq!(
            panel.figures[0],
            Figure::Seg(Seg {
                from: [0.0, 1.0],
                to: [0.5, 1.0],
                color: Some(Rgba([255, 0, 0, 255])),
                tip: Some("1".into()),
            })
        );
        assert_eq!(
            panel.figures[3],
            Figure::Rect(Rect {
                min: [0.25, 0.1],
                max: [0.5, 0.9],
                frac: true,
                color: None,
                label: Some("bd".into()),
                tip: None,
            })
        );
        assert_eq!(
            panel.figures[3].ys(),
            Vec::<f64>::new(),
            "frac has no data ys"
        );
        assert_eq!(panel.figures[2].ys(), vec![0.0, 2.0]);
    }

    #[test]
    fn a_single_panel_decodes_as_a_list_of_one() {
        let figs = decode_figures(&list(vec![id("panel")]));
        assert_eq!(figs.panels, vec![Panel::default()]);
    }

    #[test]
    fn malformed_figures_drop_with_warnings() {
        let state = list(vec![
            list(vec![
                id("panel"),
                list(vec![id("seg"), Datum::I64(0)]),
                list(vec![id("blob")]),
                list(vec![id("dot"), Datum::I64(1), Datum::I64(2)]),
            ]),
            Datum::I64(3),
        ]);
        let figs = decode_figures(&state);
        assert_eq!(figs.panels.len(), 1);
        assert_eq!(figs.panels[0].figures.len(), 1, "only the dot draws");
        assert_eq!(figs.warnings.len(), 3, "{:?}", figs.warnings);
        assert!(decode_figures(&Datum::I64(1)).panels.is_empty());
    }
}
