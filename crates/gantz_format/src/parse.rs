//! Reads `.gantz` source text into a [`Document`].
//!
//! Tokenisation is handled by [`crate::sexpr`], which wraps Steel's reader.
//! Only the registry forms `(graph ...)`, `(commits ...)` and `(names ...)`
//! are interpreted. Any other top-level form is preserved verbatim in
//! [`Document::extra`] for an extender. Embedded `expr` and `branch` code is
//! captured verbatim from its source span, so node `src` strings and the
//! content addresses that hash them are preserved byte-for-byte.

use crate::datum::{Datum, datum_from_expr};
use crate::error::{ErrorKind, FormatError};
use crate::model::{
    Addr, CommitDecl, Conn, Document, Endpoint, Form, GraphBody, GraphDef, NameDecl, NodeDecl,
    NodeSpec, RefSpec, SectionForm, SectionKey,
};
use crate::sexpr::{self, as_keyword, as_string, as_symbol, err_at, list_args, span_src};
use crate::sugar::{Sugar, SugarArgs};
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
            "section" => doc.sections.push(parse_section(&args[1..], form, src)?),
            // Preserve anything else, such as `layout`, for an extender.
            other => doc.extra.push(Form {
                head: other.to_string(),
                raw: span_src(form, src).unwrap_or_default().to_string(),
                span: sexpr::span(form).unwrap_or_default(),
            }),
        }
    }
    Ok(doc)
}

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
    Ok(NodeDecl { name, spec })
}

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
        "node" => parse_generic_spec(rest, e, src),
        other => match sugar.read_spec(other, SugarArgs::new(rest, src))? {
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
    let mut ext = None;
    let mut args = rest[1..].iter();
    while let Some(a) = args.next() {
        if as_keyword(a).as_deref() == Some("sync") {
            sync = true;
        } else if as_keyword(a).as_deref() == Some("ext") {
            // `#:ext` takes the following datum-map expr as its payload.
            ext = args.next().map(|payload| datum_from_expr(payload, src));
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
        ext,
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
    let mut fields: Vec<(String, Datum)> = Vec::new();
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
        fields.push((fname, datum_from_expr(&fargs[1], src)));
    }
    Ok(NodeSpec::Value(Datum::tagged(&tag, fields)))
}

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

/// Parse a `(section "<id>" (policy <p>) (liveness <l>) (entry <key> <datum>) ...)`
/// form. Policy and liveness are required so unknown-domain sections carry
/// their semantics through text.
fn parse_section(
    args: &[ExprKind],
    form: &ExprKind,
    src: &str,
) -> Result<SectionForm, FormatError> {
    let id = args.first().and_then(as_string).ok_or_else(|| {
        err_at(
            form,
            src,
            ErrorKind::Malformed("section requires a string id".into()),
        )
    })?;
    let mut policy = None;
    let mut liveness = None;
    let mut entries = Vec::new();
    for item in &args[1..] {
        let iargs = list_args(item).ok_or_else(|| {
            err_at(
                item,
                src,
                ErrorKind::Malformed("section item must be a list".into()),
            )
        })?;
        match iargs.first().and_then(as_symbol).as_deref() {
            Some("policy") => {
                let sym = iargs.get(1).and_then(as_symbol);
                policy = Some(match sym.as_deref() {
                    Some("keep-existing") => gantz_ca::MergePolicy::KeepExisting,
                    Some("replace") => gantz_ca::MergePolicy::Replace,
                    _ => {
                        return Err(err_at(
                            item,
                            src,
                            ErrorKind::Malformed("policy must be keep-existing or replace".into()),
                        ));
                    }
                });
            }
            Some("liveness") => {
                let sym = iargs.get(1).and_then(as_symbol);
                liveness = Some(match sym.as_deref() {
                    Some("root") => gantz_ca::Liveness::Root,
                    Some("with-name") => gantz_ca::Liveness::WithName,
                    Some("with-commit") => gantz_ca::Liveness::WithCommit,
                    Some("with-graph") => gantz_ca::Liveness::WithGraph,
                    Some("pinned") => gantz_ca::Liveness::Pinned,
                    _ => {
                        return Err(err_at(
                            item,
                            src,
                            ErrorKind::Malformed(
                                "liveness must be root, with-name, with-commit, with-graph \
                                 or pinned"
                                    .into(),
                            ),
                        ));
                    }
                });
            }
            Some("entry") => {
                if iargs.len() != 3 {
                    return Err(err_at(
                        item,
                        src,
                        ErrorKind::Malformed("entry must be (entry <key> <datum>)".into()),
                    ));
                }
                let key = parse_section_key(&iargs[1], src)?;
                let value = datum_from_expr(&iargs[2], src);
                entries.push((key, value));
            }
            _ => {
                return Err(err_at(
                    item,
                    src,
                    ErrorKind::Malformed(
                        "section item must be (policy ...), (liveness ...) or (entry ...)".into(),
                    ),
                ));
            }
        }
    }
    let policy = policy.ok_or_else(|| {
        err_at(
            form,
            src,
            ErrorKind::Malformed("section requires a (policy ...)".into()),
        )
    })?;
    let liveness = liveness.ok_or_else(|| {
        err_at(
            form,
            src,
            ErrorKind::Malformed("section requires a (liveness ...)".into()),
        )
    })?;
    Ok(SectionForm {
        id,
        policy,
        liveness,
        entries,
    })
}

/// Parse a section entry key. One of `(name <symbol>)`, `(commit "<hex>")`,
/// `(graph "<hex>")` or `(addr "<hex>")`.
fn parse_section_key(e: &ExprKind, src: &str) -> Result<SectionKey, FormatError> {
    let args = list_args(e).ok_or_else(|| {
        err_at(
            e,
            src,
            ErrorKind::Malformed("section key must be a list".into()),
        )
    })?;
    let malformed = || {
        err_at(
            e,
            src,
            ErrorKind::Malformed(
                "section key must be (name <symbol>), (commit \"<hex>\"), (graph \"<hex>\") \
                 or (addr \"<hex>\")"
                    .into(),
            ),
        )
    };
    if args.len() != 2 {
        return Err(malformed());
    }
    let kind = args.first().and_then(as_symbol).ok_or_else(malformed)?;
    match kind.as_str() {
        "name" => {
            let name = as_symbol(&args[1])
                .or_else(|| as_string(&args[1]))
                .ok_or_else(malformed)?;
            Ok(SectionKey::Name(name))
        }
        "commit" => Ok(SectionKey::Commit(
            as_string(&args[1]).ok_or_else(malformed)?,
        )),
        "graph" => Ok(SectionKey::Graph(
            as_string(&args[1]).ok_or_else(malformed)?,
        )),
        "addr" => Ok(SectionKey::Addr(as_string(&args[1]).ok_or_else(malformed)?)),
        _ => Err(malformed()),
    }
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
    let mut merge_parents = Vec::new();
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
            Some("merge-parents") => {
                merge_parents = fargs[1..]
                    .iter()
                    .map(|e| parse_addr(e, src))
                    .collect::<Result<_, _>>()?;
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
        merge_parents,
        graph,
    })
}

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

fn int_field(e: &ExprKind, src: &str) -> Result<i64, FormatError> {
    sexpr::as_i64(e, src)
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("expected an integer".into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sugar::CoreSugar;

    #[test]
    fn reads_zero_ceremony_mul() {
        let text = "\
(graph mul
  (l inlet) (r inlet) (out outlet)
  (m (expr (* $l $r)))
  (-> l (m 0)) (-> r (m 1)) (-> m out))";
        let doc = parse(text, &CoreSugar).expect("parse");
        assert_eq!(doc.graphs.len(), 1);
        let g = &doc.graphs[0];
        assert_eq!(g.id, Addr::Label("mul".to_string()));
        assert_eq!(g.body.nodes.len(), 4);
        assert_eq!(g.body.conns.len(), 3);
        // Embedded Steel code is captured verbatim.
        let m = g.body.nodes.iter().find(|n| n.name == "m").unwrap();
        match &m.spec {
            NodeSpec::Value(v) => {
                assert_eq!(v.get("type").and_then(Datum::as_str), Some("Expr"));
                assert_eq!(v.get("src").and_then(Datum::as_str), Some("(* $l $r)"));
            }
            other => panic!("expected expr value, got {other:?}"),
        }
        // `(m 1)` parses as input port 1.
        let c = &g.body.conns[1];
        assert_eq!(c.from.node, "r");
        assert_eq!(c.to.node, "m");
        assert_eq!(c.to.port, 1);
    }

    #[test]
    fn preserves_unrecognized_forms() {
        let text = "(graph mul (m (expr 1)))\n(layout mul (m 1 2))";
        let doc = parse(text, &CoreSugar).expect("parse");
        assert_eq!(doc.graphs.len(), 1);
        assert_eq!(doc.extra.len(), 1);
        assert_eq!(doc.extra[0].head, "layout");
        assert_eq!(doc.extra[0].raw, "(layout mul (m 1 2))");
    }

    #[test]
    fn descriptions_form_lands_in_extra() {
        // `(descriptions ...)` is an extender's friendly form. The gui layer
        // maps it to the `gantz.description` section, so the core parser
        // preserves it verbatim like any unrecognised form.
        let text = "\
(graph mul (m (expr 1)))
(descriptions
  (mul \"multiply two numbers\"))";
        let doc = parse(text, &CoreSugar).expect("parse");
        assert_eq!(doc.extra.len(), 1);
        assert_eq!(doc.extra[0].head, "descriptions");
    }

    #[test]
    fn round_trips_sections() {
        // A section from a domain this parser knows nothing about carries
        // its semantics as data and round-trips through a write and parse.
        let text = "\
(graph mul (m (expr 1)))
(section \"laser.palette\"
  (policy keep-existing)
  (liveness with-name)
  (entry (name mul) \"warm\"))";
        let doc = parse(text, &CoreSugar).expect("parse");
        assert_eq!(doc.sections.len(), 1);
        let section = &doc.sections[0];
        assert_eq!(section.id, "laser.palette");
        assert!(matches!(
            section.policy,
            gantz_ca::MergePolicy::KeepExisting
        ));
        assert!(matches!(section.liveness, gantz_ca::Liveness::WithName));
        assert_eq!(section.entries.len(), 1);
        assert!(matches!(&section.entries[0].0, SectionKey::Name(n) if n == "mul"));

        let written = crate::writer::write_document(&doc, &CoreSugar);
        let reparsed = parse(&written, &CoreSugar).expect("reparse");
        assert_eq!(reparsed.sections.len(), 1);
        assert_eq!(reparsed.sections[0].id, "laser.palette");
        assert_eq!(reparsed.sections[0].entries.len(), 1);
    }

    #[test]
    fn reads_keywords_branch_and_ref() {
        let text = "\
(graph g
  (s (expr (values $x (* $x 2)) #:out 2))
  (b (branch (if $n (list 0 0) (list 1 0)) \"10\" \"01\"))
  (m (ref mul \"834568e9\")))";
        let doc = parse(text, &CoreSugar).expect("parse");
        let g = &doc.graphs[0];
        let s = g.body.nodes.iter().find(|n| n.name == "s").unwrap();
        match &s.spec {
            NodeSpec::Value(v) => {
                assert_eq!(v.get("outputs").and_then(Datum::as_i64), Some(2))
            }
            _ => panic!("expected expr"),
        }
        let b = g.body.nodes.iter().find(|n| n.name == "b").unwrap();
        match &b.spec {
            NodeSpec::Value(v) => {
                assert_eq!(
                    v.get("src").and_then(Datum::as_str),
                    Some("(if $n (list 0 0) (list 1 0))")
                );
                assert_eq!(
                    v.get("branches").and_then(Datum::as_seq),
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
    /// structurally through parse, write and parse.
    #[test]
    fn generic_node_nested_map_round_trips() {
        let text = "\
(graph g
  (x (node \"Custom\"
       (cfg ((gain 6) (mode \"hi\")))
       (tags #(\"a\" \"b\"))
       (flag #t))))";
        let doc1 = parse(text, &CoreSugar).expect("parse 1");
        let node1 = &doc1.graphs[0].body.nodes[0];
        match &node1.spec {
            NodeSpec::Value(v) => {
                assert_eq!(v.get("type").and_then(Datum::as_str), Some("Custom"));
                assert!(
                    matches!(v.get("cfg"), Some(Datum::Map(_))),
                    "cfg must be a map, got {:?}",
                    v.get("cfg"),
                );
                assert!(
                    matches!(v.get("tags"), Some(Datum::Seq(_))),
                    "tags must be a seq",
                );
                assert_eq!(v.get("flag"), Some(&Datum::Bool(true)));
            }
            other => panic!("expected value, got {other:?}"),
        }
        let text2 = crate::writer::write_document(&doc1, &CoreSugar);
        let doc2 = parse(&text2, &CoreSugar).expect("parse 2");
        let node2 = &doc2.graphs[0].body.nodes[0];
        assert_eq!(
            format!("{:?}", node1.spec),
            format!("{:?}", node2.spec),
            "generic node spec must survive a write/re-parse\n--- text2 ---\n{text2}",
        );
    }
}

#[cfg(test)]
mod ref_ext_tests {
    use super::*;
    use crate::sugar::CoreSugar;

    /// A ref's `#:ext` datum-map tail parses, writes and re-parses
    /// structurally intact, and stays absent when a ref carries none.
    #[test]
    fn ref_spec_ext_round_trips() {
        let text = "\
(graph g
  (x (ref mysynth \"0000000000000000000000000000000000000000000000000000000000000000\" #:sync #:ext ((\"plyphon.dsp-ref\" ((inline #t))))))
  (y (ref plain)))";
        let expected_ext = Datum::Map(vec![(
            "plyphon.dsp-ref".to_string(),
            Datum::Map(vec![("inline".to_string(), Datum::Bool(true))]),
        )]);
        let doc1 = parse(text, &CoreSugar).expect("parse 1");
        let ref_spec = |doc: &Document, ix: usize| match &doc.graphs[0].body.nodes[ix].spec {
            NodeSpec::Ref(r) => r.clone(),
            other => panic!("expected ref, got {other:?}"),
        };
        let r1 = ref_spec(&doc1, 0);
        assert!(r1.sync);
        assert_eq!(r1.ext, Some(expected_ext.clone()));
        assert_eq!(ref_spec(&doc1, 1).ext, None);

        let text2 = crate::writer::write_document(&doc1, &CoreSugar);
        assert!(text2.contains("#:ext"));
        let doc2 = parse(&text2, &CoreSugar).expect("parse 2");
        let r2 = ref_spec(&doc2, 0);
        assert!(r2.sync);
        assert_eq!(r2.ext, Some(expected_ext));
        assert_eq!(ref_spec(&doc2, 1).ext, None);
        // A no-ext ref writes without any `#:ext` tail.
        assert_eq!(text2.matches("#:ext").count(), 1);
    }
}
