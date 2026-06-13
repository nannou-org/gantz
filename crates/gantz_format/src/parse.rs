//! Reads `.gantz` source text into a [`Document`].
//!
//! Tokenisation is handled by [`crate::sexpr`] (Steel's reader). Only the
//! registry forms - `(graph ...)`, `(commits ...)`, `(names ...)` - are
//! interpreted; any other top-level form is preserved verbatim in
//! [`Document::extra`] for an extender to interpret. Embedded `expr`/`branch`
//! code is captured verbatim from its source span so node `src` strings - and
//! the content addresses that hash them - are preserved byte-for-byte.

use crate::error::{ErrorKind, FormatError};
use crate::model::{
    Addr, CommitDecl, Conn, Document, Endpoint, Form, GraphBody, GraphDef, NameDecl, NodeDecl,
    NodeSpec, RefSpec,
};
use crate::sexpr::{self, as_keyword, as_string, as_symbol, list_args, span_src};
use serde_json::{Map, Value, json};
use steel::parser::ast::ExprKind;
use steel::parser::tokens::TokenType;

/// Parse a complete `.gantz` document.
pub fn parse(src: &str) -> Result<Document, FormatError> {
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
            "graph" => doc.graphs.push(parse_graph_def(&args[1..], form, src)?),
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

fn parse_graph_def(args: &[ExprKind], form: &ExprKind, src: &str) -> Result<GraphDef, FormatError> {
    let id_expr = args.first().ok_or_else(|| {
        err_at(
            form,
            src,
            ErrorKind::Malformed("graph requires an id".into()),
        )
    })?;
    let id = parse_addr(id_expr, src)?;
    let body = parse_graph_body(&args[1..], src)?;
    Ok(GraphDef { id, body })
}

fn parse_graph_body(items: &[ExprKind], src: &str) -> Result<GraphBody, FormatError> {
    let mut nodes = Vec::new();
    let mut conns = Vec::new();
    for item in items {
        let args = list_args(item)
            .ok_or_else(|| err_at(item, src, ErrorKind::Malformed("expected a list".into())))?;
        if args.first().and_then(as_symbol).as_deref() == Some("->") {
            conns.push(parse_conn(&args[1..], item, src)?);
        } else {
            nodes.push(parse_node_decl(args, item, src)?);
        }
    }
    Ok(GraphBody { nodes, conns })
}

fn parse_node_decl(args: &[ExprKind], item: &ExprKind, src: &str) -> Result<NodeDecl, FormatError> {
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
    let spec = parse_node_spec(&args[1], src)?;
    Ok(NodeDecl {
        name,
        index: None,
        spec,
    })
}

// -- node specs --------------------------------------------------------------

fn parse_node_spec(e: &ExprKind, src: &str) -> Result<NodeSpec, FormatError> {
    if let Some(kw) = as_symbol(e) {
        return unit_spec(&kw).ok_or_else(|| err_at(e, src, ErrorKind::UnknownNodeKeyword(kw)));
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
        "expr" => parse_expr_spec(rest, e, src),
        "branch" => parse_branch_spec(rest, e, src),
        "comment" => parse_comment_spec(rest, e, src),
        "number" => Ok(NodeSpec::Value(json!({ "type": "Number" }))),
        "log" => parse_log_spec(rest, src),
        "ref" => parse_ref_spec(false, rest, src),
        "fn-ref" => parse_ref_spec(true, rest, src),
        "graph" => Ok(NodeSpec::Graph(parse_graph_body(rest, src)?)),
        "node" => parse_generic_spec(rest, e, src),
        other => Err(err_at(
            e,
            src,
            ErrorKind::UnknownNodeKeyword(other.to_string()),
        )),
    }
}

/// A node spec written as a bare keyword (unit node), or one with a sensible
/// default (`log`).
fn unit_spec(kw: &str) -> Option<NodeSpec> {
    match kw {
        "log" => Some(NodeSpec::Value(json!({ "type": "Log", "level": "INFO" }))),
        _ => tag_for_keyword(kw).map(|tag| NodeSpec::Value(json!({ "type": tag }))),
    }
}

fn parse_expr_spec(rest: &[ExprKind], e: &ExprKind, src: &str) -> Result<NodeSpec, FormatError> {
    let code = rest
        .first()
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("expr requires code".into())))?;
    let code_src = span_src(code, src).ok_or_else(|| {
        err_at(
            code,
            src,
            ErrorKind::Malformed("could not slice expr code".into()),
        )
    })?;
    let mut obj = json!({ "type": "Expr", "src": code_src });
    if let Some(out) = keyword_int(&rest[1..], "out", src)? {
        obj["outputs"] = json!(out);
    }
    Ok(NodeSpec::Value(obj))
}

fn parse_branch_spec(rest: &[ExprKind], e: &ExprKind, src: &str) -> Result<NodeSpec, FormatError> {
    let code = rest
        .first()
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("branch requires code".into())))?;
    let code_src = span_src(code, src).ok_or_else(|| {
        err_at(
            code,
            src,
            ErrorKind::Malformed("could not slice branch code".into()),
        )
    })?;
    let masks: Vec<String> = rest[1..]
        .iter()
        .map(|m| {
            as_string(m).ok_or_else(|| {
                err_at(
                    m,
                    src,
                    ErrorKind::Malformed("branch mask must be a string".into()),
                )
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(NodeSpec::Value(
        json!({ "type": "Branch", "src": code_src, "branches": masks }),
    ))
}

fn parse_comment_spec(rest: &[ExprKind], e: &ExprKind, src: &str) -> Result<NodeSpec, FormatError> {
    let text = rest
        .first()
        .and_then(as_string)
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("comment requires text".into())))?;
    let size: [u16; 2] = match (rest.get(1), rest.get(2)) {
        (Some(w), Some(h)) => [int_field(w, src)? as u16, int_field(h, src)? as u16],
        _ => [100, 40],
    };
    Ok(NodeSpec::Value(
        json!({ "type": "Comment", "text": text, "size": size }),
    ))
}

fn parse_log_spec(rest: &[ExprKind], src: &str) -> Result<NodeSpec, FormatError> {
    let level = match rest.first().and_then(as_symbol) {
        Some(s) => log_level(&s, &rest[0], src)?,
        None => "INFO".to_string(),
    };
    Ok(NodeSpec::Value(json!({ "type": "Log", "level": level })))
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
    let mut obj = Map::new();
    obj.insert("type".to_string(), Value::String(tag));
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
        obj.insert(fname, datum_to_json(&fargs[1], src));
    }
    Ok(NodeSpec::Value(Value::Object(obj)))
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

// -- datum -> JSON (generic fallback) ----------------------------------------

fn datum_to_json(e: &ExprKind, src: &str) -> Value {
    match e {
        ExprKind::List(list) => {
            Value::Array(list.args.iter().map(|a| datum_to_json(a, src)).collect())
        }
        ExprKind::Atom(a) => match &a.syn.ty {
            TokenType::StringLiteral(s) => Value::String(s.to_string()),
            TokenType::BooleanLiteral(b) => Value::Bool(*b),
            TokenType::Number(_) => number_value(e, src),
            TokenType::Identifier(s) => match s.resolve() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                "null" => Value::Null,
                other => Value::String(other.to_string()),
            },
            TokenType::Keyword(s) => Value::String(s.resolve().to_string()),
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

fn number_value(e: &ExprKind, src: &str) -> Value {
    let Some(text) = span_src(e, src) else {
        return Value::Null;
    };
    if let Ok(i) = text.parse::<i64>() {
        Value::from(i)
    } else if let Ok(f) = text.parse::<f64>() {
        Value::from(f)
    } else {
        Value::String(text.to_string())
    }
}

// -- small wrappers over the sexpr toolkit -----------------------------------

fn int_field(e: &ExprKind, src: &str) -> Result<i64, FormatError> {
    sexpr::as_i64(e, src)
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("expected an integer".into())))
}

/// Find a `#:<key>` keyword in `args` and return the following integer value.
fn keyword_int(args: &[ExprKind], key: &str, src: &str) -> Result<Option<i64>, FormatError> {
    for (i, a) in args.iter().enumerate() {
        if as_keyword(a).as_deref() == Some(key) {
            let val = args
                .get(i + 1)
                .map(|v| int_field(v, src))
                .transpose()?
                .ok_or_else(|| {
                    err_at(
                        a,
                        src,
                        ErrorKind::Malformed(format!("#:{key} requires an integer")),
                    )
                })?;
            return Ok(Some(val));
        }
    }
    Ok(None)
}

/// Map a log-level symbol to the `log::Level` serde representation.
fn log_level(sym: &str, e: &ExprKind, src: &str) -> Result<String, FormatError> {
    match sym.to_ascii_lowercase().as_str() {
        "error" => Ok("ERROR".into()),
        "warn" => Ok("WARN".into()),
        "info" => Ok("INFO".into()),
        "debug" => Ok("DEBUG".into()),
        "trace" => Ok("TRACE".into()),
        other => Err(err_at(
            e,
            src,
            ErrorKind::Malformed(format!("unknown log level `{other}`")),
        )),
    }
}

fn tag_for_keyword(kw: &str) -> Option<&'static str> {
    crate::sugar::tag_for_keyword(kw)
}

fn err_at(e: &ExprKind, src: &str, kind: ErrorKind) -> FormatError {
    FormatError::new(kind).at(sexpr::span(e).unwrap_or_default(), src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_zero_ceremony_mul() {
        let text = "\
(graph mul
  (l inlet) (r inlet) (out outlet)
  (m (expr (* $l $r)))
  (-> l (m 0)) (-> r (m 1)) (-> m out))";
        let doc = parse(text).expect("parse");
        assert_eq!(doc.graphs.len(), 1);
        let g = &doc.graphs[0];
        assert_eq!(g.id, Addr::Label("mul".to_string()));
        assert_eq!(g.body.nodes.len(), 4);
        assert_eq!(g.body.conns.len(), 3);
        // Embedded Steel code is captured verbatim.
        let m = g.body.nodes.iter().find(|n| n.name == "m").unwrap();
        match &m.spec {
            NodeSpec::Value(v) => {
                assert_eq!(v["type"], "Expr");
                assert_eq!(v["src"], "(* $l $r)");
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
        let doc = parse(text).expect("parse");
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
        let doc = parse(text).expect("parse");
        let g = &doc.graphs[0];
        let s = g.body.nodes.iter().find(|n| n.name == "s").unwrap();
        match &s.spec {
            NodeSpec::Value(v) => assert_eq!(v["outputs"], 2),
            _ => panic!("expected expr"),
        }
        let b = g.body.nodes.iter().find(|n| n.name == "b").unwrap();
        match &b.spec {
            NodeSpec::Value(v) => {
                assert_eq!(v["src"], "(if $n (list 0 0) (list 1 0))");
                assert_eq!(v["branches"], serde_json::json!(["10", "01"]));
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
}
