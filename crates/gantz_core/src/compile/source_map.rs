//! A byte-offset map from the emitted module source text back to the graph
//! entities the compiler generated it from.
//!
//! The map is built by lexically scanning the pretty-printed module source
//! (see `vm::fmt_module`) and parsing every identifier with
//! [`names::parse`]. Scanning the *final* text - rather than threading spans
//! through emission - guarantees the offsets index the exact string that is
//! displayed and executed.
//!
//! The scanner is total: malformed input never panics, it just yields fewer
//! definitions and occurrences.

use crate::{
    compile::names::{self, Name},
    node,
};
use std::ops::Range;

/// Byte-offset spans into an emitted module's source text, resolvable to
/// node paths.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    /// Top-level forms in source order.
    defs: Vec<Def>,
    /// Identifier occurrences that parse as emitted [`Name`]s, in source
    /// order. The defining identifier of each [`Def`] is excluded (it is
    /// represented by the def itself).
    occs: Vec<Occ>,
}

/// A top-level form (almost always a `(define (name ...) ...)`).
#[derive(Clone, Debug)]
pub struct Def {
    /// The byte range of the whole form, opening to closing paren inclusive.
    pub range: Range<usize>,
    /// The defined identifier parsed as an emitted name, if it is one.
    pub name: Option<Name>,
    /// The byte range of the defined identifier (empty when the form defines
    /// nothing).
    pub name_range: Range<usize>,
}

/// One identifier occurrence within a [`Def`].
#[derive(Clone, Debug)]
pub struct Occ {
    /// The byte range of the identifier.
    pub range: Range<usize>,
    /// The parsed emitted name.
    pub name: Name,
    /// Index of the enclosing [`Def`] in [`SourceMap::defs`].
    pub def_ix: usize,
}

/// The spans associated with one node, as returned by
/// [`SourceMap::node_spans`].
#[derive(Clone, Debug, Default)]
pub struct NodeSpans {
    /// Whole-form ranges of the defs compiled *for* the node (its node fns,
    /// and for graph nodes their graph and level fns).
    pub defs: Vec<Range<usize>>,
    /// Ranges of identifiers referring to the node elsewhere (call sites and
    /// value bindings).
    pub refs: Vec<Range<usize>>,
}

impl SourceMap {
    /// Build the map by scanning the emitted module source.
    pub fn parse(src: &str) -> Self {
        scan(src)
    }

    /// The top-level forms in source order.
    pub fn defs(&self) -> &[Def] {
        &self.defs
    }

    /// All resolved identifier occurrences in source order.
    pub fn occs(&self) -> &[Occ] {
        &self.occs
    }

    /// The spans associated with the node at the given full path: the defs
    /// compiled for it and the identifiers referring to it.
    pub fn node_spans(&self, path: &[node::Id]) -> NodeSpans {
        let mut spans = NodeSpans::default();
        for def in &self.defs {
            if def_path(def) == Some(path) {
                spans.defs.push(def.range.clone());
            }
        }
        for occ in &self.occs {
            let def = &self.defs[occ.def_ix];
            if occ_path(occ, def).as_deref() == Some(path) {
                spans.refs.push(occ.range.clone());
            }
        }
        spans
    }

    /// The full path of the node best attributed to the given byte range
    /// (e.g. a steel error span).
    ///
    /// Resolution: a range covering the enclosing form's defined name (e.g.
    /// a whole-define span) attributes to the def itself; otherwise the
    /// first intersecting fn-call identifier wins, then the first
    /// intersecting value binding (resolved against the form's level), then
    /// the form's own attribution. `Some(vec![])` means "the module as a
    /// whole" (e.g. an entry fn's glue); `None` means the range lies outside
    /// every form.
    pub fn node_at(&self, range: Range<usize>) -> Option<Vec<node::Id>> {
        // Treat an empty range as a point query.
        let range = range.start..range.end.max(range.start + 1);
        let ix = self
            .defs
            .partition_point(|d| d.range.start <= range.start)
            .checked_sub(1)?;
        let def = &self.defs[ix];
        if range.start >= def.range.end {
            return None;
        }
        let intersects = |r: &Range<usize>| r.start < range.end && range.start < r.end;
        if intersects(&def.name_range) {
            return Some(def_path(def).map(<[_]>::to_vec).unwrap_or_default());
        }

        let mut first_var: Option<&Occ> = None;
        for occ in self.occs.iter().filter(|o| o.def_ix == ix) {
            if !intersects(&occ.range) {
                continue;
            }
            match occ.name {
                Name::NodeFn { .. } | Name::GraphFn { .. } | Name::LvlFn { .. } => {
                    return occ_path(occ, def);
                }
                _ => first_var = first_var.or(Some(occ)),
            }
        }
        if let Some(path) = first_var.and_then(|occ| occ_path(occ, def)) {
            return Some(path);
        }
        Some(def_path(def).map(<[_]>::to_vec).unwrap_or_default())
    }
}

/// The full path of the node a def was compiled for, if any.
fn def_path(def: &Def) -> Option<&[node::Id]> {
    match def.name.as_ref()? {
        Name::NodeFn { path, .. } | Name::GraphFn { path, .. } | Name::LvlFn { path, .. } => {
            Some(path)
        }
        _ => None,
    }
}

/// The graph level whose node ids are in scope within a def's body.
fn level_path(def: &Def) -> Option<&[node::Id]> {
    match def.name.as_ref()? {
        // A node fn's body is the node's own expr: relative ids refer to
        // its siblings.
        Name::NodeFn { path, .. } => path.split_last().map(|(_, parent)| parent),
        // Graph and level fn bodies are lowered from the level's interior.
        Name::GraphFn { path, .. } | Name::LvlFn { path, .. } => Some(path),
        Name::EntryFn { .. } => Some(&[]),
        _ => None,
    }
}

/// The full path of the node an occurrence refers to, resolving
/// level-relative ids against the enclosing def.
fn occ_path(occ: &Occ, def: &Def) -> Option<Vec<node::Id>> {
    match &occ.name {
        Name::NodeFn { path, .. } | Name::GraphFn { path, .. } | Name::LvlFn { path, .. } => {
            Some(path.clone())
        }
        Name::EntryFn { .. } => Some(vec![]),
        Name::Output { node, .. }
        | Name::Input { node, .. }
        | Name::Result { node }
        | Name::Join { id: node } => {
            let mut path = level_path(def)?.to_vec();
            path.push(*node);
            Some(path)
        }
    }
}

/// Lexically scan the module source into defs and occurrences.
fn scan(src: &str) -> SourceMap {
    /// The state of the currently open top-level form.
    struct Form {
        start: usize,
        head_seen: bool,
        is_define: bool,
        name: Option<(Range<usize>, Option<Name>)>,
    }

    fn close(form: Form, end: usize) -> Def {
        let (name_range, name) = match form.name {
            Some((range, name)) => (range, name),
            None => (form.start..form.start, None),
        };
        Def {
            range: form.start..end,
            name,
            name_range,
        }
    }

    let bytes = src.as_bytes();
    let mut map = SourceMap::default();
    let mut form: Option<Form> = None;
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' => {
                if depth == 0 {
                    form = Some(Form {
                        start: i,
                        head_seen: false,
                        is_define: false,
                        name: None,
                    });
                }
                depth += 1;
                i += 1;
            }
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(form) = form.take() {
                        map.defs.push(close(form, i + 1));
                    }
                }
                i += 1;
            }
            b'"' => i = skip_string(bytes, i),
            b';' => i = skip_comment(bytes, i),
            b'\'' | b'`' | b',' => i += 1,
            b if b.is_ascii_whitespace() => i += 1,
            _ => {
                let start = i;
                i = scan_atom(bytes, i);
                let Some(form) = form.as_mut() else {
                    continue;
                };
                let token = &src[start..i];
                if !form.head_seen {
                    form.head_seen = true;
                    form.is_define = token == "define";
                } else if form.is_define && form.name.is_none() {
                    form.name = Some((start..i, names::parse(token)));
                } else if let Some(name) = names::parse(token) {
                    map.occs.push(Occ {
                        range: start..i,
                        name,
                        def_ix: map.defs.len(),
                    });
                }
            }
        }
    }
    // Close an unbalanced trailing form so occ def indices stay valid.
    if let Some(form) = form.take() {
        map.defs.push(close(form, bytes.len()));
    }
    map
}

/// Advance past a string literal starting at the opening quote.
fn skip_string(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Advance past a `;` comment to the end of the line.
fn skip_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

/// Advance past one atom token starting at `start`.
fn scan_atom(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() {
        // A char literal's payload is consumed unconditionally so that
        // `#\(` and friends cannot unbalance the scan.
        if i == start + 2 && &bytes[start..i] == br"#\" {
            i += 1;
            continue;
        }
        match bytes[i] {
            b'(' | b')' | b'[' | b']' | b'"' | b';' | b'\'' | b'`' | b',' => break,
            b if b.is_ascii_whitespace() => break,
            _ => i += 1,
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_defs_and_occs() {
        let src = "(define (node-fn-0:1-i1-o1 input0) (+ input0 1))\n\n\
                   (define (graph-fn-0) (node-fn-0:1-i1-o1 node-1-o0))";
        let map = SourceMap::parse(src);
        assert_eq!(map.defs().len(), 2);
        assert_eq!(
            map.defs()[0].name,
            Some(Name::NodeFn {
                path: vec![0, 1],
                inputs: "1".parse().unwrap(),
                outputs: "1".parse().unwrap(),
            })
        );
        assert_eq!(&src[map.defs()[0].name_range.clone()], "node-fn-0:1-i1-o1");
        assert_eq!(
            map.defs()[1].name,
            Some(Name::GraphFn {
                path: vec![0],
                inputs: node::Conns::empty(),
            })
        );
        // The second def holds two occs: the node fn call and the var.
        let occs: Vec<_> = map
            .occs()
            .iter()
            .map(|o| (&src[o.range.clone()], o.def_ix))
            .collect();
        assert_eq!(occs, vec![("node-fn-0:1-i1-o1", 1), ("node-1-o0", 1)]);
    }

    #[test]
    fn scanner_skips_strings_comments_and_char_literals() {
        let src = "(define (node-fn-0-o1) (foo \"a ) b \\\" (\" #\\( #\\) ; node-1-o0 )\n 'node-2-o0 sym))";
        let map = SourceMap::parse(src);
        assert_eq!(map.defs().len(), 1);
        assert_eq!(&src[map.defs()[0].range.clone()], src);
        // Only the quoted (but still identifier-shaped) var is recorded; the
        // string, char literals and comment contribute nothing.
        let occs: Vec<_> = map.occs().iter().map(|o| &src[o.range.clone()]).collect();
        assert_eq!(occs, vec!["node-2-o0"]);
    }

    #[test]
    fn node_at_resolution() {
        let src = "(define (lvl-fn-abababab-3) (define node-0-o0 (node-fn-3:0-o1)) node-0-o0)";
        let map = SourceMap::parse(src);
        // The call identifier resolves to the called node's full path.
        let call = src.find("(node-fn-3:0-o1)").unwrap();
        assert_eq!(map.node_at(call..call + 16), Some(vec![3, 0]));
        // A var resolves relative to the def's level path.
        let var = src.rfind("node-0-o0").unwrap();
        assert_eq!(map.node_at(var..var + 9), Some(vec![3, 0]));
        // A span with no identifiers falls back to the def's attribution.
        let kw = src.find("define").unwrap();
        assert_eq!(map.node_at(kw..kw + 6), Some(vec![3]));
        // Out of range.
        assert_eq!(map.node_at(src.len()..src.len() + 1), None);
    }
}
