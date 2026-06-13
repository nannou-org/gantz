//! Reads `.gantz` source text into a [`Document`].
//!
//! Tokenisation is handled by [`crate::sexpr`] (Steel's reader). Only the
//! registry forms - `(graph ...)`, `(commits ...)`, `(names ...)` - are
//! interpreted; any other top-level form is preserved verbatim in
//! [`Document::extra`] for an extender to interpret. Embedded `expr`/`branch`
//! code is captured verbatim from its source span so node `src` strings - and
//! the content addresses that hash them - are preserved byte-for-byte.

use crate::datum::{Datum, datum_from_expr};
use crate::error::{ErrorKind, FormatError};
use crate::model::{
    Addr, CommitDecl, Conn, Document, Endpoint, Form, GraphBody, GraphDef, NameDecl, NodeDecl,
    NodeSpec, RefSpec,
};
use crate::sexpr::{self, as_keyword, as_string, as_symbol, list_args, span_src};
use crate::sugar::Sugar;
use steel::parser::ast::ExprKind;

/// Parse a complete `.gantz` document, interpreting node sugar with `sugar`.
pub fn parse(src: &str, sugar: &dyn Sugar) -> Result<Document, FormatError> {
    let forms = sexpr::read(src)?;
    let mut doc = Document::default();
    for form in &forms {
        let args = list_args(form)
            .ok_or_else(|| err_at(form, src, ErrorKind::Malformed("expected a list".into())))?;
        let head = args.first().and_then(as_symbol).ok_or_else(|| {
            err_at(
                form,
                src,
                ErrorKind::Malformed("expected a head keyword".into()),
            )
        })?;
        match head.as_str() {
            "graph" => doc
                .graphs
                .push(parse_graph_def(&args[1..], form, src, sugar)?),
            "commits" => doc.commits.extend(parse_commits_table(&args[1..], src)?),
            "names" => doc.names.extend(parse_names_table(&args[1..], src)?),
            // Preserve anything else (e.g. `layout`, `demo`) for an extender.
            other => doc.extra.push(Form {
                head: other.to_string(),
                raw: span_src(form, src).unwrap_or_default().to_string(),
                span: sexpr::span(form).unwrap_or_default(),
            }),
        }
    }
    Ok(doc)
}

// -- top-level forms ---------------------------------------------------------

fn parse_graph_def(
    args: &[ExprKind],
    form: &ExprKind,
    src: &str,
    sugar: &dyn Sugar,
) -> Result<GraphDef, FormatError> {
    let id_expr = args.first().ok_or_else(|| {
        err_at(
            form,
            src,
            ErrorKind::Malformed("graph requires an id".into()),
        )
    })?;
    let id = parse_addr(id_expr, src)?;
    let body = parse_graph_body(&args[1..], src, sugar)?;
    Ok(GraphDef { id, body })
}

fn parse_graph_body(
    items: &[ExprKind],
    src: &str,
    sugar: &dyn Sugar,
) -> Result<GraphBody, FormatError> {
    let mut nodes = Vec::new();
    let mut conns = Vec::new();
    for item in items {
        let args = list_args(item)
            .ok_or_else(|| err_at(item, src, ErrorKind::Malformed("expected a list".into())))?;
        if args.first().and_then(as_symbol).as_deref() == Some("->") {
            conns.push(parse_conn(&args[1..], item, src)?);
        } else {
            nodes.push(parse_node_decl(args, item, src, sugar)?);
        }
    }
    Ok(GraphBody { nodes, conns })
}

fn parse_node_decl(
    args: &[ExprKind],
    item: &ExprKind,
    src: &str,
    sugar: &dyn Sugar,
) -> Result<NodeDecl, FormatError> {
    if args.len() != 2 {
        return Err(err_at(
            item,
            src,
            ErrorKind::Malformed("node must be (name spec)".into()),
        ));
    }
    let name = as_symbol(&args[0]).ok_or_else(|| {
        err_at(
            &args[0],
            src,
            ErrorKind::Malformed("node name must be a symbol".into()),
        )
    })?;
    let spec = parse_node_spec(&args[1], src, sugar)?;
    Ok(NodeDecl {
        name,
        index: None,
        spec,
    })
}

// -- node specs --------------------------------------------------------------

/// Reserved core heads matched before any sugar, so a sugar cannot shadow them.
fn parse_node_spec(e: &ExprKind, src: &str, sugar: &dyn Sugar) -> Result<NodeSpec, FormatError> {
    if let Some(kw) = as_symbol(e) {
        return sugar
            .read_bare(&kw)
            .map(NodeSpec::Value)
            .ok_or_else(|| err_at(e, src, ErrorKind::UnknownNodeKeyword(kw)));
    }
    let args = list_args(e).ok_or_else(|| {
        err_at(
            e,
            src,
            ErrorKind::Malformed("node spec must be a keyword or list".into()),
        )
    })?;
    let head = args.first().and_then(as_symbol).ok_or_else(|| {
        err_at(
            e,
            src,
            ErrorKind::Malformed("node spec needs a keyword".into()),
        )
    })?;
    let rest = &args[1..];
    match head.as_str() {
        "ref" => parse_ref_spec(false, rest, src),
        "fn-ref" => parse_ref_spec(true, rest, src),
        "graph" => Ok(NodeSpec::Graph(parse_graph_body(rest, src, sugar)?)),
        "node" => parse_generic_spec(rest, e, src),
        other => match sugar.read_spec(other, rest, src)? {
            Some(datum) => Ok(NodeSpec::Value(datum)),
            None => Err(err_at(
                e,
                src,
                ErrorKind::UnknownNodeKeyword(other.to_string()),
            )),
        },
    }
}

fn parse_ref_spec(func: bool, rest: &[ExprKind], src: &str) -> Result<NodeSpec, FormatError> {
    let name = rest
        .first()
        .and_then(as_symbol)
        .ok_or_else(|| match rest.first() {
            Some(e) => err_at(e, src, ErrorKind::Malformed("ref requires a name".into())),
            None => FormatError::new(ErrorKind::Malformed("ref requires a name".into())),
        })?;
    let mut addr = None;
    let mut sync = false;
    for a in &rest[1..] {
        if as_keyword(a).as_deref() == Some("sync") {
            sync = true;
        } else if let Some(s) = as_string(a) {
            addr = Some(Addr::Concrete(s));
        } else if let Some(s) = as_symbol(a) {
            addr = Some(Addr::Label(s));
        }
    }
    Ok(NodeSpec::Ref(RefSpec {
        func,
        name,
        addr,
        sync,
    }))
}

fn parse_generic_spec(rest: &[ExprKind], e: &ExprKind, src: &str) -> Result<NodeSpec, FormatError> {
    let tag = rest.first().and_then(as_string).ok_or_else(|| {
        err_at(
            e,
            src,
            ErrorKind::Malformed("node requires a type string".into()),
        )
    })?;
    let mut entries = vec![("type".to_string(), Datum::Str(tag))];
    for field in &rest[1..] {
        let fargs = list_args(field).ok_or_else(|| {
            err_at(
                field,
                src,
                ErrorKind::Malformed("node field must be (name value)".into()),
            )
        })?;
        if fargs.len() != 2 {
            return Err(err_at(
                field,
                src,
                ErrorKind::Malformed("node field must be (name value)".into()),
            ));
        }
        let fname = as_symbol(&fargs[0]).ok_or_else(|| {
            err_at(
                &fargs[0],
                src,
                ErrorKind::Malformed("field name must be a symbol".into()),
            )
        })?;
        entries.push((fname, datum_from_expr(&fargs[1], src)));
    }
    Ok(NodeSpec::Value(Datum::Map(entries)))
}

// -- connections / commits / names -------------------------------------------

fn parse_conn(args: &[ExprKind], item: &ExprKind, src: &str) -> Result<Conn, FormatError> {
    if args.len() != 2 {
        return Err(err_at(
            item,
            src,
            ErrorKind::Malformed("connection must be (-> from to)".into()),
        ));
    }
    Ok(Conn {
        from: parse_endpoint(&args[0], src)?,
        to: parse_endpoint(&args[1], src)?,
    })
}

fn parse_endpoint(e: &ExprKind, src: &str) -> Result<Endpoint, FormatError> {
    if let Some(node) = as_symbol(e) {
        return Ok(Endpoint { node, port: 0 });
    }
    let args = list_args(e).ok_or_else(|| {
        err_at(
            e,
            src,
            ErrorKind::Malformed("endpoint must be a name or (name port)".into()),
        )
    })?;
    let node = args.first().and_then(as_symbol).ok_or_else(|| {
        err_at(
            e,
            src,
            ErrorKind::Malformed("endpoint name must be a symbol".into()),
        )
    })?;
    let port = args
        .get(1)
        .map(|p| int_field(p, src))
        .transpose()?
        .unwrap_or(0) as u16;
    Ok(Endpoint { node, port })
}

fn parse_commits_table(args: &[ExprKind], src: &str) -> Result<Vec<CommitDecl>, FormatError> {
    let mut commits = Vec::new();
    for item in args {
        let iargs = list_args(item).ok_or_else(|| {
            err_at(
                item,
                src,
                ErrorKind::Malformed("commit entry must be a list".into()),
            )
        })?;
        commits.push(parse_commit_entry(iargs, item, src)?);
    }
    Ok(commits)
}

fn parse_names_table(args: &[ExprKind], src: &str) -> Result<Vec<NameDecl>, FormatError> {
    let mut names = Vec::new();
    for item in args {
        let iargs = list_args(item).ok_or_else(|| {
            err_at(
                item,
                src,
                ErrorKind::Malformed("name entry must be (name commit)".into()),
            )
        })?;
        if iargs.len() != 2 {
            return Err(err_at(
                item,
                src,
                ErrorKind::Malformed("name entry must be (name commit)".into()),
            ));
        }
        let name = as_symbol(&iargs[0]).ok_or_else(|| {
            err_at(
                &iargs[0],
                src,
                ErrorKind::Malformed("name must be a symbol".into()),
            )
        })?;
        let commit = parse_addr(&iargs[1], src)?;
        names.push(NameDecl { name, commit });
    }
    Ok(names)
}

fn parse_commit_entry(
    args: &[ExprKind],
    item: &ExprKind,
    src: &str,
) -> Result<CommitDecl, FormatError> {
    let id = args
        .first()
        .map(|e| parse_addr(e, src))
        .transpose()?
        .ok_or_else(|| {
            err_at(
                item,
                src,
                ErrorKind::Malformed("commit requires an id".into()),
            )
        })?;
    let mut secs = 0u64;
    let mut nanos = 0u32;
    let mut parent = None;
    let mut graph = None;
    for field in &args[1..] {
        let fargs = list_args(field).ok_or_else(|| {
            err_at(
                field,
                src,
                ErrorKind::Malformed("commit field must be a list".into()),
            )
        })?;
        match fargs.first().and_then(as_symbol).as_deref() {
            Some("time") => {
                secs = fargs
                    .get(1)
                    .map(|n| int_field(n, src))
                    .transpose()?
                    .unwrap_or(0) as u64;
                nanos = fargs
                    .get(2)
                    .map(|n| int_field(n, src))
                    .transpose()?
                    .unwrap_or(0) as u32;
            }
            Some("parent") => {
                parent = match fargs.get(1) {
                    Some(e) if as_symbol(e).as_deref() == Some("none") => None,
                    Some(e) => Some(parse_addr(e, src)?),
                    None => None,
                };
            }
            Some("graph") => {
                graph = fargs.get(1).map(|e| parse_addr(e, src)).transpose()?;
            }
            _ => {
                return Err(err_at(
                    field,
                    src,
                    ErrorKind::Malformed("unknown commit field".into()),
                ));
            }
        }
    }
    let graph = graph.ok_or_else(|| {
        err_at(
            item,
            src,
            ErrorKind::Malformed("commit requires a graph".into()),
        )
    })?;
    Ok(CommitDecl {
        id,
        secs,
        nanos,
        parent,
        graph,
    })
}

// -- addresses ---------------------------------------------------------------

fn parse_addr(e: &ExprKind, src: &str) -> Result<Addr, FormatError> {
    if let Some(s) = as_string(e) {
        Ok(Addr::Concrete(s))
    } else if let Some(s) = as_symbol(e) {
        Ok(Addr::Label(s))
    } else {
        Err(err_at(
            e,
            src,
            ErrorKind::BadAddr("expected a string or label".into()),
        ))
    }
}

// -- small wrappers over the sexpr toolkit -----------------------------------

fn int_field(e: &ExprKind, src: &str) -> Result<i64, FormatError> {
    sexpr::as_i64(e, src)
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("expected an integer".into())))
}

fn err_at(e: &ExprKind, src: &str, kind: ErrorKind) -> FormatError {
    FormatError::new(kind).at(sexpr::span(e).unwrap_or_default(), src)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datum::{datum_field, datum_int, datum_seq, datum_str};
    use crate::sugar::DefaultSugar;

    #[test]
    fn reads_zero_ceremony_mul() {
        let text = "\
(graph mul
  (l inlet) (r inlet) (out outlet)
  (m (expr (* $l $r)))
  (-> l (m 0)) (-> r (m 1)) (-> m out))";
        let doc = parse(text, &DefaultSugar).expect("parse");
        assert_eq!(doc.graphs.len(), 1);
        let g = &doc.graphs[0];
        assert_eq!(g.id, Addr::Label("mul".to_string()));
        assert_eq!(g.body.nodes.len(), 4);
        assert_eq!(g.body.conns.len(), 3);
        // Embedded Steel code is captured verbatim.
        let m = g.body.nodes.iter().find(|n| n.name == "m").unwrap();
        match &m.spec {
            NodeSpec::Value(v) => {
                assert_eq!(datum_field(v, "type").and_then(datum_str), Some("Expr"));
                assert_eq!(datum_field(v, "src").and_then(datum_str), Some("(* $l $r)"));
            }
            other => panic!("expected expr value, got {other:?}"),
        }
        // Ports parse: `(m 1)` -> input port 1.
        let c = &g.body.conns[1];
        assert_eq!(c.from.node, "r");
        assert_eq!(c.to.node, "m");
        assert_eq!(c.to.port, 1);
    }

    #[test]
    fn preserves_unrecognized_forms() {
        let text = "(graph mul (m (expr 1)))\n(layout mul (m 1 2))";
        let doc = parse(text, &DefaultSugar).expect("parse");
        assert_eq!(doc.graphs.len(), 1);
        assert_eq!(doc.extra.len(), 1);
        assert_eq!(doc.extra[0].head, "layout");
        assert_eq!(doc.extra[0].raw, "(layout mul (m 1 2))");
    }

    #[test]
    fn reads_keywords_branch_and_ref() {
        let text = "\
(graph g
  (s (expr (values $x (* $x 2)) #:out 2))
  (b (branch (if $n (list 0 0) (list 1 0)) \"10\" \"01\"))
  (m (ref mul \"834568e9\")))";
        let doc = parse(text, &DefaultSugar).expect("parse");
        let g = &doc.graphs[0];
        let s = g.body.nodes.iter().find(|n| n.name == "s").unwrap();
        match &s.spec {
            NodeSpec::Value(v) => {
                assert_eq!(datum_field(v, "outputs").and_then(datum_int), Some(2))
            }
            _ => panic!("expected expr"),
        }
        let b = g.body.nodes.iter().find(|n| n.name == "b").unwrap();
        match &b.spec {
            NodeSpec::Value(v) => {
                assert_eq!(
                    datum_field(v, "src").and_then(datum_str),
                    Some("(if $n (list 0 0) (list 1 0))")
                );
                assert_eq!(
                    datum_field(v, "branches").and_then(datum_seq),
                    Some(&[Datum::Str("10".into()), Datum::Str("01".into())][..])
                );
            }
            _ => panic!("expected branch"),
        }
        let m = g.body.nodes.iter().find(|n| n.name == "m").unwrap();
        match &m.spec {
            NodeSpec::Ref(r) => {
                assert!(!r.func);
                assert_eq!(r.name, "mul");
                assert_eq!(r.addr, Some(Addr::Concrete("834568e9".to_string())));
            }
            _ => panic!("expected ref"),
        }
    }

    /// A generic `(node ...)` whose fields nest a map and a seq round-trips
    /// structurally through parse -> write -> parse (the lossy-object bug fix).
    #[test]
    fn generic_node_nested_map_round_trips() {
        let text = "\
(graph g
  (x (node \"Custom\"
       (cfg ((gain 6) (mode \"hi\")))
       (tags #(\"a\" \"b\"))
       (flag #t))))";
        let doc1 = parse(text, &DefaultSugar).expect("parse 1");
        let node1 = &doc1.graphs[0].body.nodes[0];
        match &node1.spec {
            NodeSpec::Value(v) => {
                assert_eq!(datum_field(v, "type").and_then(datum_str), Some("Custom"));
                // The lossy bug turned this nested object into an array; it must
                // stay a map.
                assert!(
                    matches!(datum_field(v, "cfg"), Some(Datum::Map(_))),
                    "cfg must be a map, got {:?}",
                    datum_field(v, "cfg"),
                );
                assert!(
                    matches!(datum_field(v, "tags"), Some(Datum::Seq(_))),
                    "tags must be a seq",
                );
                assert_eq!(datum_field(v, "flag"), Some(&Datum::Bool(true)));
            }
            other => panic!("expected value, got {other:?}"),
        }
        let text2 = crate::writer::write_document(&doc1, &DefaultSugar);
        let doc2 = parse(&text2, &DefaultSugar).expect("parse 2");
        let node2 = &doc2.graphs[0].body.nodes[0];
        assert_eq!(
            format!("{:?}", node1.spec),
            format!("{:?}", node2.spec),
            "generic node spec must survive a write/re-parse\n--- text2 ---\n{text2}",
        );
    }
}
