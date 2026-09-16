//! The total decoder from the abstract value model to the typed tree.
//!
//! [`decode`] never fails. Malformed subtrees become inline
//! [`Element::Error`] nodes. Recoverable issues become [`Warning`]s. See
//! [`crate::diag`] for the boundary rule.
//!
//! One tolerance rule spans every codec. `Datum` has no symbol variant, so
//! identifier positions also accept strings, and text positions also accept
//! identifiers. Identifier positions are tags, attribute names and enum-like
//! attribute values.

use crate::diag::{ErrorReason, TreePath, Warning, WarningKind};
use crate::elem::{
    ATTRS_MARKER, Align, BindPath, Button, Col, Dialer, DialerStyle, Element, ErrorElem, Frame,
    Grid, Key, Label, Matrix, Plot, PlotMode, PlotStyle, RESERVED_TAGS, RefGui, Rgba, Row, Scope,
    Sep, Space, Toggle, Value,
};
use crate::sexpr::{self, SExpr};

/// Caps that bound pathological computed trees. The defaults are generous.
/// A hand-authored GUI never meets them. A runaway computed one stays
/// visible as an inline error instead of stalling the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// The maximum nesting depth, with the root element at depth zero.
    pub max_depth: usize,
    /// The maximum total number of elements.
    pub max_elements: usize,
}

/// The result of a total decode.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    /// The decoded tree.
    pub root: Element,
    /// Recoverable issues encountered along the way.
    pub warnings: Vec<Warning>,
}

/// Decode state threaded through the recursion.
struct Ctx<'a> {
    limits: &'a Limits,
    remaining: usize,
    warnings: Vec<Warning>,
}

/// The attribute entries of one element, consumed by name.
struct AttrSet {
    entries: Vec<(String, Option<SExpr>)>,
}

/// Typed attribute extraction for one element, warning on mismatches and
/// falling back to defaults.
struct Attrs<'a, 'b> {
    ctx: &'a mut Ctx<'b>,
    path: &'a [usize],
    tag: &'a str,
    set: AttrSet,
}

/// Every tag of the v1 vocabulary.
const KNOWN_TAGS: &[&str] = &[
    "col", "row", "grid", "frame", "sep", "space", "scope", "dialer", "toggle", "button", "matrix",
    "label", "value", "plot", "ref-gui",
];

impl Ctx<'_> {
    fn warn(&mut self, path: &[usize], kind: WarningKind) {
        let path = TreePath(path.to_vec());
        self.warnings.push(Warning { path, kind });
    }
}

impl AttrSet {
    fn empty() -> Self {
        AttrSet {
            entries: Vec::new(),
        }
    }

    fn take(&mut self, name: &str) -> Option<SExpr> {
        self.entries
            .iter_mut()
            .find(|(n, v)| n == name && v.is_some())
            .and_then(|(_, v)| v.take())
    }

    fn present(&self, name: &str) -> bool {
        self.entries.iter().any(|(n, _)| n == name)
    }
}

impl<'a, 'b> Attrs<'a, 'b> {
    fn new(ctx: &'a mut Ctx<'b>, path: &'a [usize], tag: &'a str, set: AttrSet) -> Self {
        Attrs {
            ctx,
            path,
            tag,
            set,
        }
    }

    /// Consume the attribute `name`, parse it, and warn with a fallback to
    /// the field default on a mismatch.
    fn take<T>(
        &mut self,
        name: &str,
        expected: &'static str,
        parse: impl Fn(&SExpr) -> Option<T>,
    ) -> Option<T> {
        let v = self.set.take(name)?;
        let parsed = parse(&v);
        if parsed.is_none() {
            let kind = WarningKind::InvalidAttrValue {
                tag: self.tag.to_string(),
                attr: name.to_string(),
                expected,
                found: sexpr::summary(&v),
            };
            self.ctx.warn(self.path, kind);
        }
        parsed
    }

    fn f32(&mut self, name: &str) -> Option<f32> {
        self.take(name, "a number", as_f32)
    }

    fn f64(&mut self, name: &str) -> Option<f64> {
        self.take(name, "a number", as_f64)
    }

    fn u8(&mut self, name: &str) -> Option<u8> {
        self.take(name, "an integer in 0..=255", as_u8)
    }

    fn bool_or(&mut self, name: &str, default: bool) -> bool {
        self.take(name, "a boolean", as_bool).unwrap_or(default)
    }

    fn string(&mut self, name: &str) -> Option<String> {
        self.take(name, "a string", text)
    }

    fn color(&mut self, name: &str) -> Option<Rgba> {
        self.take(name, "a colour string like \"#rrggbb\"", |v| {
            text(v).as_deref().and_then(Rgba::parse)
        })
    }

    fn key(&mut self) -> Option<Key> {
        self.take("key", "a string or an integer", as_key)
    }

    fn bind(&mut self) -> Option<BindPath> {
        self.take("bind", "a list of node ids", as_bind)
    }

    /// The required `cols` attribute of `grid`. It substitutes 1 with a
    /// warning when absent.
    fn cols(&mut self) -> u32 {
        if self.set.present("cols") {
            self.take("cols", "a positive integer", as_cols)
                .unwrap_or(1)
        } else {
            let kind = WarningKind::MissingAttr {
                tag: self.tag.to_string(),
                attr: "cols".to_string(),
                default: "1".to_string(),
            };
            self.ctx.warn(self.path, kind);
            1
        }
    }

    /// Warn for every attribute never consumed.
    fn finish(self) {
        let Attrs {
            ctx,
            path,
            tag,
            set,
            ..
        } = self;
        for (name, v) in set.entries {
            if v.is_some() {
                let kind = WarningKind::UnknownAttr {
                    tag: tag.to_string(),
                    attr: name,
                };
                ctx.warn(path, kind);
            }
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_depth: 64,
            max_elements: 10_000,
        }
    }
}

/// Decode a UI tree from the abstract value model.
///
/// Total. This never fails. Malformed subtrees become inline
/// [`Element::Error`] nodes that keep their slot in the tree. Recoverable
/// issues become [`Warning`]s and the element renders with documented
/// defaults.
pub fn decode(expr: SExpr, limits: &Limits) -> Decoded {
    let mut ctx = Ctx {
        limits,
        remaining: limits.max_elements,
        warnings: Vec::new(),
    };
    let mut path = Vec::new();
    let root = elem(&mut ctx, &mut path, 0, expr);
    Decoded {
        root,
        warnings: ctx.warnings,
    }
}

fn error_at(path: &[usize], reason: ErrorReason) -> Element {
    Element::Error(ErrorElem {
        path: TreePath(path.to_vec()),
        reason,
    })
}

/// Decode one element form.
fn elem(ctx: &mut Ctx, path: &mut Vec<usize>, depth: usize, expr: SExpr) -> Element {
    if ctx.remaining == 0 {
        return error_at(path, ErrorReason::ElementLimit(ctx.limits.max_elements));
    }
    if depth > ctx.limits.max_depth {
        return error_at(path, ErrorReason::DepthLimit(ctx.limits.max_depth));
    }
    ctx.remaining -= 1;

    let SExpr::List(items) = expr else {
        let found = sexpr::summary(&expr);
        return error_at(path, ErrorReason::NotAnElement { found });
    };
    let mut items = items.into_iter();
    let Some(head) = items.next() else {
        let found = "an empty list".to_string();
        return error_at(path, ErrorReason::NotAnElement { found });
    };
    let Some(tag) = text(&head) else {
        let found = format!("a list headed by {}", sexpr::summary(&head));
        return error_at(path, ErrorReason::NotAnElement { found });
    };
    if tag == ATTRS_MARKER {
        return error_at(path, ErrorReason::MisplacedAttrs);
    }
    if RESERVED_TAGS.contains(&tag.as_str()) {
        return error_at(path, ErrorReason::ReservedTag(tag));
    }
    if !KNOWN_TAGS.contains(&tag.as_str()) {
        return error_at(path, ErrorReason::UnknownTag(tag));
    }

    let (attr_entries, rest) = split_attrs(items.collect());
    let set = match attr_entries {
        Some(entries) => parse_attrs(ctx, path, &tag, entries),
        None => AttrSet::empty(),
    };

    match tag.as_str() {
        "col" | "row" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let gap = a.f32("gap");
            let align = a.take("align", "one of start, center, end", |v| {
                text(v).as_deref().and_then(Align::from_name)
            });
            let key = a.key();
            a.finish();
            let children = children(ctx, path, depth, rest);
            match tag.as_str() {
                "col" => Element::Col(Col {
                    gap,
                    align,
                    key,
                    children,
                }),
                _ => Element::Row(Row {
                    gap,
                    align,
                    key,
                    children,
                }),
            }
        }
        "grid" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let cols = a.cols();
            let gap = a.f32("gap");
            let key = a.key();
            a.finish();
            let children = children(ctx, path, depth, rest);
            Element::Grid(Grid {
                cols,
                gap,
                key,
                children,
            })
        }
        "frame" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let title = a.string("title");
            let key = a.key();
            a.finish();
            let children = children(ctx, path, depth, rest);
            Element::Frame(Frame {
                title,
                key,
                children,
            })
        }
        "sep" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let key = a.key();
            a.finish();
            warn_ignored(ctx, path, &tag, rest.len());
            Element::Sep(Sep { key })
        }
        "space" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let key = a.key();
            a.finish();
            let mut items = rest.into_iter();
            let amount = match items.next() {
                None => None,
                Some(v) => match as_f32(&v) {
                    Some(f) => Some(f),
                    None => {
                        let kind = WarningKind::InvalidArg {
                            tag: tag.clone(),
                            what: "a numeric amount",
                            found: sexpr::summary(&v),
                        };
                        ctx.warn(path, kind);
                        None
                    }
                },
            };
            warn_ignored(ctx, path, &tag, items.count());
            Element::Space(Space { amount, key })
        }
        "scope" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let key = a.key();
            a.finish();
            let mut items = rest.into_iter();
            let id = match node_id_arg(&tag, items.next()) {
                Ok(id) => id,
                Err(reason) => return error_at(path, reason),
            };
            let children = children(ctx, path, depth, items.collect());
            Element::Scope(Scope { id, key, children })
        }
        "dialer" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let bind = a.bind();
            let min = a.f64("min");
            let max = a.f64("max");
            let step = a.f64("step");
            let precision = a.u8("precision");
            let label = a.string("label");
            let push = a.bool_or("push", true);
            let style = a.take("style", "one of slider, knob", |v| {
                text(v).as_deref().and_then(DialerStyle::from_name)
            });
            let key = a.key();
            a.finish();
            warn_ignored(ctx, path, &tag, rest.len());
            Element::Dialer(Dialer {
                bind,
                min,
                max,
                step,
                precision,
                label,
                push,
                style,
                key,
            })
        }
        "toggle" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let bind = a.bind();
            let label = a.string("label");
            let push = a.bool_or("push", true);
            let key = a.key();
            a.finish();
            warn_ignored(ctx, path, &tag, rest.len());
            Element::Toggle(Toggle {
                bind,
                label,
                push,
                key,
            })
        }
        "button" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let bind = a.bind();
            let label = a.string("label");
            let key = a.key();
            a.finish();
            warn_ignored(ctx, path, &tag, rest.len());
            Element::Button(Button { bind, label, key })
        }
        "matrix" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let bind = a.bind();
            let cell_size = a.f32("cell-size");
            let push = a.bool_or("push", true);
            let key = a.key();
            a.finish();
            warn_ignored(ctx, path, &tag, rest.len());
            Element::Matrix(Matrix {
                bind,
                cell_size,
                push,
                key,
            })
        }
        "label" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let size = a.f32("size");
            let color = a.color("color");
            let key = a.key();
            a.finish();
            let mut items = rest.into_iter();
            let text_arg = match items.next() {
                None => {
                    let kind = WarningKind::MissingText {
                        tag: tag.clone(),
                        what: "a text string",
                    };
                    ctx.warn(path, kind);
                    String::new()
                }
                Some(v) => match text(&v) {
                    Some(t) => t,
                    None => {
                        let kind = WarningKind::InvalidArg {
                            tag: tag.clone(),
                            what: "a text string",
                            found: sexpr::summary(&v),
                        };
                        ctx.warn(path, kind);
                        String::new()
                    }
                },
            };
            warn_ignored(ctx, path, &tag, items.count());
            Element::Label(Label {
                text: text_arg,
                size,
                color,
                key,
            })
        }
        "value" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let bind = a.bind();
            let wrap = a.bool_or("wrap", false);
            let key = a.key();
            a.finish();
            warn_ignored(ctx, path, &tag, rest.len());
            Element::Value(Value { bind, wrap, key })
        }
        "plot" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let bind = a.bind();
            let mode = a.take("mode", "one of scope, signal", |v| {
                text(v).as_deref().and_then(PlotMode::from_name)
            });
            let style = a.take("style", "one of bars, line", |v| {
                text(v).as_deref().and_then(PlotStyle::from_name)
            });
            let color = a.color("color");
            let grid = a.bool_or("grid", false);
            let axes = a.bool_or("axes", false);
            let interactive = a.bool_or("interactive", false);
            let y_min = a.f32("y-min");
            let y_max = a.f32("y-max");
            let w = a.f32("w");
            let h = a.f32("h");
            let key = a.key();
            a.finish();
            warn_ignored(ctx, path, &tag, rest.len());
            Element::Plot(Plot {
                bind,
                mode,
                style,
                color,
                grid,
                axes,
                interactive,
                y_min,
                y_max,
                w,
                h,
                key,
            })
        }
        "ref-gui" => {
            let mut a = Attrs::new(ctx, path, &tag, set);
            let key = a.key();
            a.finish();
            let mut items = rest.into_iter();
            let id = match node_id_arg(&tag, items.next()) {
                Ok(id) => id,
                Err(reason) => return error_at(path, reason),
            };
            warn_ignored(ctx, path, &tag, items.count());
            Element::RefGui(RefGui { id, key })
        }
        // Unreachable. Membership in KNOWN_TAGS is checked above.
        _ => error_at(path, ErrorReason::UnknownTag(tag)),
    }
}

/// Decode the child elements of a container, dropping remaining siblings
/// behind a single visible error once the element budget is spent.
fn children(ctx: &mut Ctx, path: &mut Vec<usize>, depth: usize, items: Vec<SExpr>) -> Vec<Element> {
    let mut elems = Vec::with_capacity(items.len());
    for (ix, item) in items.into_iter().enumerate() {
        path.push(ix);
        if ctx.remaining == 0 {
            elems.push(error_at(
                path,
                ErrorReason::ElementLimit(ctx.limits.max_elements),
            ));
            path.pop();
            break;
        }
        elems.push(elem(ctx, path, depth + 1, item));
        path.pop();
    }
    elems
}

/// Split a leading attribute block from an element's remaining items.
fn split_attrs(rest: Vec<SExpr>) -> (Option<Vec<SExpr>>, Vec<SExpr>) {
    let mut iter = rest.into_iter();
    match iter.next() {
        Some(SExpr::List(entries)) if entries.first().is_some_and(is_attrs_marker) => {
            (Some(entries), iter.collect())
        }
        Some(first) => (None, std::iter::once(first).chain(iter).collect()),
        None => (None, Vec::new()),
    }
}

/// Whether the value is the reserved `@` marker.
fn is_attrs_marker(expr: &SExpr) -> bool {
    matches!(expr, SExpr::Ident(s) | SExpr::Str(s) if s == ATTRS_MARKER)
}

/// Parse the entries of an attribute block, marker included, into an
/// [`AttrSet`]. Warn for malformed entries and duplicates.
fn parse_attrs(ctx: &mut Ctx, path: &[usize], tag: &str, entries: Vec<SExpr>) -> AttrSet {
    let mut parsed: Vec<(String, Option<SExpr>)> = Vec::new();
    for entry in entries.into_iter().skip(1) {
        match entry {
            SExpr::List(pair) => match <[SExpr; 2]>::try_from(pair) {
                Ok([name_expr, value]) => match text(&name_expr) {
                    Some(name) if parsed.iter().any(|(n, _)| *n == name) => {
                        let kind = WarningKind::DuplicateAttr {
                            tag: tag.to_string(),
                            attr: name,
                        };
                        ctx.warn(path, kind);
                    }
                    Some(name) => parsed.push((name, Some(value))),
                    None => {
                        let kind = WarningKind::MalformedAttr {
                            tag: tag.to_string(),
                            found: sexpr::summary(&name_expr),
                        };
                        ctx.warn(path, kind);
                    }
                },
                Err(pair) => {
                    let kind = WarningKind::MalformedAttr {
                        tag: tag.to_string(),
                        found: sexpr::summary(&SExpr::List(pair)),
                    };
                    ctx.warn(path, kind);
                }
            },
            other => {
                let kind = WarningKind::MalformedAttr {
                    tag: tag.to_string(),
                    found: sexpr::summary(&other),
                };
                ctx.warn(path, kind);
            }
        }
    }
    AttrSet { entries: parsed }
}

/// Extract the required leading node id argument of `scope` and `ref-gui`.
fn node_id_arg(tag: &str, arg: Option<SExpr>) -> Result<usize, ErrorReason> {
    let missing = |found: String| ErrorReason::MissingArg {
        tag: tag.to_string(),
        what: "a node id",
        found,
    };
    match arg {
        None => Err(missing("nothing".to_string())),
        Some(v) => as_node_id(&v).ok_or_else(|| missing(sexpr::summary(&v))),
    }
}

fn warn_ignored(ctx: &mut Ctx, path: &[usize], tag: &str, count: usize) {
    if count > 0 {
        let kind = WarningKind::IgnoredChildren {
            tag: tag.to_string(),
            count,
        };
        ctx.warn(path, kind);
    }
}

/// The contents of an identifier or string.
///
/// This is the tolerance rule in one place. Identifier positions accept
/// strings and text positions accept identifiers, because `Datum` encodes
/// identifiers as strings.
fn text(expr: &SExpr) -> Option<String> {
    match expr {
        SExpr::Ident(s) | SExpr::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// A number. Integers widen to floats, never the reverse.
fn as_f64(expr: &SExpr) -> Option<f64> {
    match expr {
        SExpr::Float(f) => Some(*f),
        SExpr::Int(i) => Some(*i as f64),
        _ => None,
    }
}

fn as_f32(expr: &SExpr) -> Option<f32> {
    as_f64(expr).map(|f| f as f32)
}

fn as_u8(expr: &SExpr) -> Option<u8> {
    match expr {
        SExpr::Int(i) => u8::try_from(*i).ok(),
        _ => None,
    }
}

/// A grid column count, a positive integer.
fn as_cols(expr: &SExpr) -> Option<u32> {
    match expr {
        SExpr::Int(i) if *i >= 1 => u32::try_from(*i).ok(),
        _ => None,
    }
}

fn as_bool(expr: &SExpr) -> Option<bool> {
    match expr {
        SExpr::Bool(b) => Some(*b),
        _ => None,
    }
}

fn as_key(expr: &SExpr) -> Option<Key> {
    match expr {
        SExpr::Ident(s) | SExpr::Str(s) => Some(Key::Str(s.clone())),
        SExpr::Int(i) => Some(Key::Int(*i)),
        _ => None,
    }
}

/// A node id, a non-negative integer.
fn as_node_id(expr: &SExpr) -> Option<usize> {
    match expr {
        SExpr::Int(i) => usize::try_from(*i).ok(),
        _ => None,
    }
}

/// A binding path, a list of node ids. Machine produced, so no bare integer
/// sugar.
fn as_bind(expr: &SExpr) -> Option<BindPath> {
    match expr {
        SExpr::List(items) => items
            .iter()
            .map(as_node_id)
            .collect::<Option<Vec<_>>>()
            .map(BindPath),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(s: &str) -> SExpr {
        SExpr::Ident(s.to_string())
    }

    fn str_(s: &str) -> SExpr {
        SExpr::Str(s.to_string())
    }

    fn int(i: i64) -> SExpr {
        SExpr::Int(i)
    }

    fn float(f: f64) -> SExpr {
        SExpr::Float(f)
    }

    fn list(items: Vec<SExpr>) -> SExpr {
        SExpr::List(items)
    }

    fn attr(name: &str, value: SExpr) -> SExpr {
        list(vec![ident(name), value])
    }

    fn attrs(entries: Vec<SExpr>) -> SExpr {
        let mut items = vec![ident(ATTRS_MARKER)];
        items.extend(entries);
        list(items)
    }

    fn dec(expr: SExpr) -> Decoded {
        decode(expr, &Limits::default())
    }

    fn error_reason(elem: &Element) -> &ErrorReason {
        match elem {
            Element::Error(e) => &e.reason,
            other => panic!("expected an error element, got {other:?}"),
        }
    }

    #[test]
    fn col_and_row_decode_with_attrs_and_children() {
        let d = dec(list(vec![
            ident("col"),
            attrs(vec![attr("gap", int(4)), attr("align", ident("center"))]),
            list(vec![ident("row"), list(vec![ident("sep")])]),
        ]));
        let expected = Element::Col(Col {
            gap: Some(4.0),
            align: Some(Align::Center),
            key: None,
            children: vec![Element::Row(Row {
                children: vec![Element::Sep(Sep::default())],
                ..Default::default()
            })],
        });
        assert_eq!(d.root, expected);
        assert_eq!(d.warnings, vec![]);
    }

    #[test]
    fn every_control_decodes_fully_attributed() {
        let d = dec(list(vec![
            ident("dialer"),
            attrs(vec![
                attr("bind", list(vec![int(0), int(2)])),
                attr("min", float(0.0)),
                attr("max", float(1.0)),
                attr("step", float(0.1)),
                attr("precision", int(3)),
                attr("label", str_("cutoff")),
                attr("push", SExpr::Bool(false)),
                attr("style", ident("knob")),
                attr("key", str_("k")),
            ]),
        ]));
        let expected = Element::Dialer(Dialer {
            bind: Some(BindPath(vec![0, 2])),
            min: Some(0.0),
            max: Some(1.0),
            step: Some(0.1),
            precision: Some(3),
            label: Some("cutoff".to_string()),
            push: false,
            style: Some(DialerStyle::Knob),
            key: Some(Key::Str("k".to_string())),
        });
        assert_eq!(d.root, expected);
        assert_eq!(d.warnings, vec![]);

        let d = dec(list(vec![
            ident("toggle"),
            attrs(vec![
                attr("bind", list(vec![int(5)])),
                attr("label", str_("drive")),
            ]),
        ]));
        let expected = Element::Toggle(Toggle {
            bind: Some(BindPath(vec![5])),
            label: Some("drive".to_string()),
            ..Default::default()
        });
        assert_eq!(d.root, expected);

        let d = dec(list(vec![
            ident("button"),
            attrs(vec![attr("bind", list(vec![int(7)]))]),
        ]));
        let expected = Element::Button(Button {
            bind: Some(BindPath(vec![7])),
            ..Default::default()
        });
        assert_eq!(d.root, expected);

        let d = dec(list(vec![
            ident("matrix"),
            attrs(vec![
                attr("bind", list(vec![int(1)])),
                attr("cell-size", int(12)),
                attr("push", SExpr::Bool(false)),
            ]),
        ]));
        let expected = Element::Matrix(Matrix {
            bind: Some(BindPath(vec![1])),
            cell_size: Some(12.0),
            push: false,
            key: None,
        });
        assert_eq!(d.root, expected);
    }

    #[test]
    fn controls_apply_documented_defaults() {
        assert_eq!(
            dec(list(vec![ident("dialer")])).root,
            Element::Dialer(Dialer::default())
        );
        assert_eq!(
            dec(list(vec![ident("toggle")])).root,
            Element::Toggle(Toggle::default())
        );
        assert_eq!(
            dec(list(vec![ident("value")])).root,
            Element::Value(Value::default())
        );
        assert_eq!(
            dec(list(vec![ident("plot")])).root,
            Element::Plot(Plot::default())
        );
    }

    #[test]
    fn display_elements_decode() {
        let d = dec(list(vec![
            ident("label"),
            attrs(vec![
                attr("size", float(18.0)),
                attr("color", str_("#ff0080")),
            ]),
            str_("hello"),
        ]));
        let expected = Element::Label(Label {
            text: "hello".to_string(),
            size: Some(18.0),
            color: Some(Rgba([255, 0, 128, 255])),
            key: None,
        });
        assert_eq!(d.root, expected);
        assert_eq!(d.warnings, vec![]);

        let d = dec(list(vec![
            ident("value"),
            attrs(vec![
                attr("bind", list(vec![int(3)])),
                attr("wrap", SExpr::Bool(true)),
            ]),
        ]));
        let expected = Element::Value(Value {
            bind: Some(BindPath(vec![3])),
            wrap: true,
            key: None,
        });
        assert_eq!(d.root, expected);

        let d = dec(list(vec![
            ident("plot"),
            attrs(vec![
                attr("bind", list(vec![int(4)])),
                attr("mode", ident("scope")),
                attr("style", ident("line")),
                attr("color", str_("#00ff0080")),
                attr("grid", SExpr::Bool(true)),
                attr("axes", SExpr::Bool(true)),
                attr("interactive", SExpr::Bool(true)),
                attr("y-min", float(-1.0)),
                attr("y-max", float(1.0)),
                attr("w", int(120)),
                attr("h", int(80)),
            ]),
        ]));
        let expected = Element::Plot(Plot {
            bind: Some(BindPath(vec![4])),
            mode: Some(PlotMode::Scope),
            style: Some(PlotStyle::Line),
            color: Some(Rgba([0, 255, 0, 128])),
            grid: true,
            axes: true,
            interactive: true,
            y_min: Some(-1.0),
            y_max: Some(1.0),
            w: Some(120.0),
            h: Some(80.0),
            key: None,
        });
        assert_eq!(d.root, expected);
        assert_eq!(d.warnings, vec![]);
    }

    #[test]
    fn frame_and_grid_decode() {
        let d = dec(list(vec![
            ident("frame"),
            attrs(vec![attr("title", str_("filter"))]),
            list(vec![ident("sep")]),
        ]));
        let expected = Element::Frame(Frame {
            title: Some("filter".to_string()),
            key: None,
            children: vec![Element::Sep(Sep::default())],
        });
        assert_eq!(d.root, expected);

        let d = dec(list(vec![
            ident("grid"),
            attrs(vec![attr("cols", int(3)), attr("gap", int(2))]),
            list(vec![ident("sep")]),
        ]));
        let expected = Element::Grid(Grid {
            cols: 3,
            gap: Some(2.0),
            key: None,
            children: vec![Element::Sep(Sep::default())],
        });
        assert_eq!(d.root, expected);
        assert_eq!(d.warnings, vec![]);
    }

    #[test]
    fn grid_without_cols_defaults_with_warning() {
        let d = dec(list(vec![ident("grid")]));
        assert_eq!(d.root, Element::Grid(Grid::default()));
        assert_eq!(
            d.warnings,
            vec![Warning {
                path: TreePath(vec![]),
                kind: WarningKind::MissingAttr {
                    tag: "grid".to_string(),
                    attr: "cols".to_string(),
                    default: "1".to_string(),
                },
            }]
        );
    }

    #[test]
    fn space_amount_variants() {
        assert_eq!(
            dec(list(vec![ident("space"), int(8)])).root,
            Element::Space(Space {
                amount: Some(8.0),
                key: None,
            })
        );
        assert_eq!(
            dec(list(vec![ident("space"), float(8.5)])).root,
            Element::Space(Space {
                amount: Some(8.5),
                key: None,
            })
        );
        assert_eq!(
            dec(list(vec![ident("space")])).root,
            Element::Space(Space::default())
        );
        let d = dec(list(vec![ident("space"), str_("wide")]));
        assert_eq!(d.root, Element::Space(Space::default()));
        assert!(matches!(d.warnings[0].kind, WarningKind::InvalidArg { .. }));
    }

    #[test]
    fn scope_decodes_and_prefixes_children() {
        let d = dec(list(vec![ident("scope"), int(3), list(vec![ident("sep")])]));
        let expected = Element::Scope(Scope {
            id: 3,
            key: None,
            children: vec![Element::Sep(Sep::default())],
        });
        assert_eq!(d.root, expected);
        assert_eq!(d.warnings, vec![]);
    }

    #[test]
    fn scope_and_ref_gui_require_a_node_id() {
        let d = dec(list(vec![ident("scope")]));
        assert!(matches!(
            error_reason(&d.root),
            ErrorReason::MissingArg { tag, .. } if tag == "scope"
        ));

        let d = dec(list(vec![ident("scope"), str_("nope")]));
        assert!(matches!(
            error_reason(&d.root),
            ErrorReason::MissingArg { .. }
        ));

        let d = dec(list(vec![ident("ref-gui"), int(-1)]));
        assert!(matches!(
            error_reason(&d.root),
            ErrorReason::MissingArg { .. }
        ));

        let d = dec(list(vec![ident("ref-gui"), int(9)]));
        assert_eq!(d.root, Element::RefGui(RefGui { id: 9, key: None }));
    }

    #[test]
    fn unknown_attr_warns_and_is_ignored() {
        let d = dec(list(vec![
            ident("dialer"),
            attrs(vec![attr("wobble", int(1))]),
        ]));
        assert_eq!(d.root, Element::Dialer(Dialer::default()));
        assert_eq!(
            d.warnings,
            vec![Warning {
                path: TreePath(vec![]),
                kind: WarningKind::UnknownAttr {
                    tag: "dialer".to_string(),
                    attr: "wobble".to_string(),
                },
            }]
        );
    }

    #[test]
    fn duplicate_attr_keeps_first_and_warns() {
        let d = dec(list(vec![
            ident("dialer"),
            attrs(vec![attr("min", float(1.0)), attr("min", float(9.0))]),
        ]));
        let Element::Dialer(dialer) = &d.root else {
            panic!("expected a dialer");
        };
        assert_eq!(dialer.min, Some(1.0));
        assert!(matches!(
            d.warnings[0].kind,
            WarningKind::DuplicateAttr { .. }
        ));
    }

    #[test]
    fn invalid_attr_value_falls_back_with_warning() {
        let d = dec(list(vec![
            ident("dialer"),
            attrs(vec![attr("min", str_("low")), attr("push", int(1))]),
        ]));
        assert_eq!(d.root, Element::Dialer(Dialer::default()));
        assert_eq!(d.warnings.len(), 2);
        assert!(
            d.warnings
                .iter()
                .all(|w| matches!(w.kind, WarningKind::InvalidAttrValue { .. }))
        );
    }

    #[test]
    fn ints_widen_to_float_attrs_but_floats_never_narrow() {
        let d = dec(list(vec![
            ident("dialer"),
            attrs(vec![attr("min", int(2)), attr("precision", float(2.5))]),
        ]));
        let Element::Dialer(dialer) = &d.root else {
            panic!("expected a dialer");
        };
        assert_eq!(dialer.min, Some(2.0));
        assert_eq!(dialer.precision, None);
        assert!(matches!(
            d.warnings[0].kind,
            WarningKind::InvalidAttrValue { ref attr, .. } if attr == "precision"
        ));

        let d = dec(list(vec![
            ident("grid"),
            attrs(vec![attr("cols", float(2.5))]),
        ]));
        let Element::Grid(grid) = &d.root else {
            panic!("expected a grid");
        };
        assert_eq!(grid.cols, 1);
    }

    #[test]
    fn malformed_attr_entries_warn_and_skip() {
        let d = dec(list(vec![
            ident("dialer"),
            attrs(vec![
                int(3),
                list(vec![ident("min")]),
                list(vec![int(1), int(2)]),
                attr("max", float(1.0)),
            ]),
        ]));
        let Element::Dialer(dialer) = &d.root else {
            panic!("expected a dialer");
        };
        assert_eq!(dialer.max, Some(1.0));
        assert_eq!(d.warnings.len(), 3);
        assert!(
            d.warnings
                .iter()
                .all(|w| matches!(w.kind, WarningKind::MalformedAttr { .. }))
        );
    }

    #[test]
    fn keys_accept_string_and_int() {
        let d = dec(list(vec![
            ident("sep"),
            attrs(vec![attr("key", str_("a"))]),
        ]));
        assert_eq!(
            d.root,
            Element::Sep(Sep {
                key: Some(Key::Str("a".to_string())),
            })
        );
        let d = dec(list(vec![ident("sep"), attrs(vec![attr("key", int(7))])]));
        assert_eq!(
            d.root,
            Element::Sep(Sep {
                key: Some(Key::Int(7)),
            })
        );
        let d = dec(list(vec![
            ident("sep"),
            attrs(vec![attr("key", SExpr::Bool(true))]),
        ]));
        assert_eq!(d.root, Element::Sep(Sep::default()));
        assert!(matches!(
            d.warnings[0].kind,
            WarningKind::InvalidAttrValue { .. }
        ));
    }

    #[test]
    fn bind_requires_a_list_of_node_ids() {
        let ok = dec(list(vec![
            ident("value"),
            attrs(vec![attr("bind", list(vec![int(1), int(2)]))]),
        ]));
        let Element::Value(value) = &ok.root else {
            panic!("expected a value");
        };
        assert_eq!(value.bind, Some(BindPath(vec![1, 2])));

        for bad in [
            attr("bind", int(1)),
            attr("bind", list(vec![int(-1)])),
            attr("bind", list(vec![str_("x")])),
        ] {
            let d = dec(list(vec![ident("value"), attrs(vec![bad])]));
            let Element::Value(value) = &d.root else {
                panic!("expected a value");
            };
            assert_eq!(value.bind, None);
            assert!(matches!(
                d.warnings[0].kind,
                WarningKind::InvalidAttrValue { ref attr, .. } if attr == "bind"
            ));
        }
    }

    #[test]
    fn bad_colors_warn_and_fall_back() {
        for bad in ["ff0080", "#ff008", "#gg0080"] {
            let d = dec(list(vec![
                ident("label"),
                attrs(vec![attr("color", str_(bad))]),
                str_("x"),
            ]));
            let Element::Label(label) = &d.root else {
                panic!("expected a label");
            };
            assert_eq!(label.color, None);
            assert!(matches!(
                d.warnings[0].kind,
                WarningKind::InvalidAttrValue { ref attr, .. } if attr == "color"
            ));
        }
    }

    #[test]
    fn identifier_positions_accept_strings_and_vice_versa() {
        // A tag as a string, an enum value as a string, text as an ident.
        let d = dec(list(vec![
            str_("col"),
            attrs(vec![attr("align", str_("center"))]),
            list(vec![str_("label"), ident("hi")]),
        ]));
        let expected = Element::Col(Col {
            align: Some(Align::Center),
            children: vec![Element::Label(Label {
                text: "hi".to_string(),
                ..Default::default()
            })],
            ..Default::default()
        });
        assert_eq!(d.root, expected);
        assert_eq!(d.warnings, vec![]);
    }

    #[test]
    fn label_text_variants() {
        let d = dec(list(vec![ident("label")]));
        assert_eq!(d.root, Element::Label(Label::default()));
        assert!(matches!(
            d.warnings[0].kind,
            WarningKind::MissingText { .. }
        ));

        let d = dec(list(vec![ident("label"), int(3)]));
        assert_eq!(d.root, Element::Label(Label::default()));
        assert!(matches!(d.warnings[0].kind, WarningKind::InvalidArg { .. }));

        let d = dec(list(vec![ident("label"), str_("x"), str_("y")]));
        assert!(matches!(
            d.warnings[0].kind,
            WarningKind::IgnoredChildren { count: 1, .. }
        ));
    }

    #[test]
    fn leaves_ignore_children_with_a_warning() {
        let d = dec(list(vec![ident("sep"), list(vec![ident("sep")]), int(2)]));
        assert_eq!(d.root, Element::Sep(Sep::default()));
        assert_eq!(
            d.warnings,
            vec![Warning {
                path: TreePath(vec![]),
                kind: WarningKind::IgnoredChildren {
                    tag: "sep".to_string(),
                    count: 2,
                },
            }]
        );
    }

    #[test]
    fn unknown_tag_errors_at_its_path() {
        let d = dec(list(vec![
            ident("col"),
            list(vec![ident("sep")]),
            list(vec![ident("bogus"), int(1)]),
        ]));
        let Element::Col(col) = &d.root else {
            panic!("expected a col");
        };
        assert_eq!(col.children[0], Element::Sep(Sep::default()));
        assert_eq!(
            col.children[1],
            Element::Error(ErrorElem {
                path: TreePath(vec![1]),
                reason: ErrorReason::UnknownTag("bogus".to_string()),
            })
        );
    }

    #[test]
    fn every_reserved_tag_errors() {
        for tag in RESERVED_TAGS {
            let d = dec(list(vec![ident(tag)]));
            assert_eq!(
                *error_reason(&d.root),
                ErrorReason::ReservedTag(tag.to_string()),
            );
        }
    }

    #[test]
    fn siblings_survive_a_malformed_child() {
        let d = dec(list(vec![
            ident("col"),
            list(vec![ident("dialer")]),
            list(vec![ident("bogus")]),
            list(vec![ident("toggle")]),
        ]));
        let Element::Col(col) = &d.root else {
            panic!("expected a col");
        };
        assert_eq!(col.children.len(), 3);
        assert_eq!(col.children[0], Element::Dialer(Dialer::default()));
        assert!(matches!(col.children[1], Element::Error(_)));
        assert_eq!(col.children[2], Element::Toggle(Toggle::default()));
        assert_eq!(d.warnings, vec![]);
    }

    #[test]
    fn nested_error_paths_are_exact() {
        let d = dec(list(vec![
            ident("col"),
            list(vec![ident("sep")]),
            list(vec![
                ident("row"),
                list(vec![ident("sep")]),
                list(vec![ident("frame"), int(5)]),
            ]),
        ]));
        let Element::Col(col) = &d.root else {
            panic!("expected a col");
        };
        let Element::Row(row) = &col.children[1] else {
            panic!("expected a row");
        };
        let Element::Frame(frame) = &row.children[1] else {
            panic!("expected a frame");
        };
        assert_eq!(
            frame.children[0],
            Element::Error(ErrorElem {
                path: TreePath(vec![1, 1, 0]),
                reason: ErrorReason::NotAnElement {
                    found: "the integer `5`".to_string(),
                },
            })
        );
    }

    #[test]
    fn misplaced_attr_blocks_error() {
        // An attribute block in child position.
        let d = dec(list(vec![
            ident("col"),
            list(vec![ident("sep")]),
            attrs(vec![attr("gap", int(4))]),
        ]));
        let Element::Col(col) = &d.root else {
            panic!("expected a col");
        };
        assert_eq!(*error_reason(&col.children[1]), ErrorReason::MisplacedAttrs);

        // An attribute block as the root form.
        let d = dec(attrs(vec![attr("gap", int(4))]));
        assert_eq!(*error_reason(&d.root), ErrorReason::MisplacedAttrs);

        // The marker as a string still counts.
        let d = dec(list(vec![str_("@"), attr("gap", int(4))]));
        assert_eq!(*error_reason(&d.root), ErrorReason::MisplacedAttrs);
    }

    #[test]
    fn non_element_roots_error() {
        assert!(matches!(
            error_reason(&dec(int(3)).root),
            ErrorReason::NotAnElement { .. }
        ));
        assert!(matches!(
            error_reason(&dec(list(vec![])).root),
            ErrorReason::NotAnElement { .. }
        ));
        let d = dec(list(vec![list(vec![ident("col")])]));
        assert!(matches!(
            error_reason(&d.root),
            ErrorReason::NotAnElement { .. }
        ));
    }

    #[test]
    fn foreign_values_error_with_their_summary() {
        let d = dec(list(vec![
            ident("col"),
            SExpr::Other("a closure".to_string()),
        ]));
        let Element::Col(col) = &d.root else {
            panic!("expected a col");
        };
        assert_eq!(
            *error_reason(&col.children[0]),
            ErrorReason::NotAnElement {
                found: "a closure".to_string(),
            }
        );
    }

    #[test]
    fn depth_limit_replaces_the_offending_subtree() {
        let limits = Limits {
            max_depth: 2,
            ..Default::default()
        };
        let at_limit = list(vec![
            ident("col"),
            list(vec![ident("col"), list(vec![ident("sep")])]),
        ]);
        let d = decode(at_limit, &limits);
        assert_eq!(d.warnings, vec![]);
        assert!(!tree_has_error(&d.root));

        let past_limit = list(vec![
            ident("col"),
            list(vec![
                ident("col"),
                list(vec![ident("col"), list(vec![ident("sep")])]),
            ]),
        ]);
        let d = decode(past_limit, &limits);
        let Element::Col(c0) = &d.root else {
            panic!("expected a col");
        };
        let Element::Col(c1) = &c0.children[0] else {
            panic!("expected a col");
        };
        let Element::Col(c2) = &c1.children[0] else {
            panic!("expected a col");
        };
        assert_eq!(
            c2.children[0],
            Element::Error(ErrorElem {
                path: TreePath(vec![0, 0, 0]),
                reason: ErrorReason::DepthLimit(2),
            })
        );
    }

    #[test]
    fn element_limit_truncates_visibly() {
        let limits = Limits {
            max_elements: 3,
            ..Default::default()
        };
        let d = decode(
            list(vec![
                ident("col"),
                list(vec![ident("sep")]),
                list(vec![ident("sep")]),
                list(vec![ident("sep")]),
                list(vec![ident("sep")]),
            ]),
            &limits,
        );
        let Element::Col(col) = &d.root else {
            panic!("expected a col");
        };
        assert_eq!(col.children.len(), 3);
        assert_eq!(col.children[0], Element::Sep(Sep::default()));
        assert_eq!(col.children[1], Element::Sep(Sep::default()));
        assert_eq!(
            col.children[2],
            Element::Error(ErrorElem {
                path: TreePath(vec![2]),
                reason: ErrorReason::ElementLimit(3),
            })
        );
    }

    fn tree_has_error(elem: &Element) -> bool {
        matches!(elem, Element::Error(_)) || elem.children().iter().any(tree_has_error)
    }
}
