//! Reads `.gantz` source text into a [`File`] AST.
//!
//! The document is parsed with Steel's own reader
//! ([`steel::parser::parser::Parser::parse_without_lowering`]) so that all
//! tokenisation (strings, keywords, numbers, identifiers like `->` and `$l`,
//! and arbitrary embedded Steel code) matches Steel exactly, and special forms
//! are left as plain lists. Embedded `expr`/`branch` code is captured verbatim
//! from its source span so node `src` strings - and the content addresses that
//! hash them - are preserved byte-for-byte.

use super::error::{ErrorKind, FormatError, Span};
use super::model::{
    Addr, CommitDecl, Conn, Demo, Endpoint, File, GraphBody, GraphDef, Layout, NameDecl, NodeDecl,
    NodeSpec, RefSpec,
};
use super::sugar::tag_for_keyword;
use serde_json::{Map, Value, json};
use steel::parser::ast::ExprKind;
use steel::parser::parser::Parser;
use steel::parser::tokens::TokenType;

/// Parse a complete `.gantz` document into a [`File`] AST.
pub fn parse_file(src: &str) -> Result<File, FormatError> {
    let forms = Parser::parse_without_lowering(src)
        .map_err(|e| FormatError::new(ErrorKind::Read(e.to_string())))?;
    let mut file = File::default();
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
            "graph" => file.graphs.push(parse_graph_def(&args[1..], form, src)?),
            "layout" => file.layouts.push(parse_layout(&args[1..], form, src)?),
            "commits" => file.commits.extend(parse_commits_table(&args[1..], src)?),
            "names" => file.names.extend(parse_names_table(&args[1..], src)?),
            "demo" => file.demos.push(parse_demo(&args[1..], form, src)?),
            other => return Err(err_at(form, src, ErrorKind::UnknownForm(other.to_string()))),
        }
    }
    Ok(file)
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
    let name = rest.first().and_then(as_symbol).ok_or_else(|| {
        let e = rest.first();
        match e {
            Some(e) => err_at(e, src, ErrorKind::Malformed("ref requires a name".into())),
            None => FormatError::new(ErrorKind::Malformed("ref requires a name".into())),
        }
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

// -- connections / layout / commits / names / demo ---------------------------

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

fn parse_layout(args: &[ExprKind], form: &ExprKind, src: &str) -> Result<Layout, FormatError> {
    let id_expr = args.first().ok_or_else(|| {
        err_at(
            form,
            src,
            ErrorKind::Malformed("layout requires a graph id".into()),
        )
    })?;
    let graph = parse_addr(id_expr, src)?;
    let mut positions = Vec::new();
    let mut scene = None;
    for item in &args[1..] {
        let iargs = list_args(item)
            .ok_or_else(|| err_at(item, src, ErrorKind::Malformed("expected a list".into())))?;
        match iargs.first().and_then(as_symbol).as_deref() {
            Some("scene") => {
                let f: Vec<f32> = iargs[1..]
                    .iter()
                    .map(|n| float_field(n, src))
                    .collect::<Result<_, _>>()?;
                if f.len() != 4 {
                    return Err(err_at(
                        item,
                        src,
                        ErrorKind::Malformed("scene needs 4 numbers".into()),
                    ));
                }
                scene = Some([f[0], f[1], f[2], f[3]]);
            }
            Some(name) => {
                let x = iargs.get(1).map(|n| float_field(n, src)).transpose()?;
                let y = iargs.get(2).map(|n| float_field(n, src)).transpose()?;
                match (x, y) {
                    (Some(x), Some(y)) => positions.push((name.to_string(), x, y)),
                    _ => {
                        return Err(err_at(
                            item,
                            src,
                            ErrorKind::Malformed("position must be (name x y)".into()),
                        ));
                    }
                }
            }
            None => {
                return Err(err_at(
                    item,
                    src,
                    ErrorKind::Malformed("layout entry needs a head".into()),
                ));
            }
        }
    }
    Ok(Layout {
        graph,
        positions,
        scene,
    })
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

fn parse_demo(args: &[ExprKind], form: &ExprKind, src: &str) -> Result<Demo, FormatError> {
    let graph = args.first().and_then(as_symbol).ok_or_else(|| {
        err_at(
            form,
            src,
            ErrorKind::Malformed("demo requires a name".into()),
        )
    })?;
    let demo = args.get(1).and_then(as_string).ok_or_else(|| {
        err_at(
            form,
            src,
            ErrorKind::Malformed("demo requires a demo name".into()),
        )
    })?;
    Ok(Demo { graph, demo })
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

// -- atom / list accessors ---------------------------------------------------

fn list_args(e: &ExprKind) -> Option<&[ExprKind]> {
    match e {
        ExprKind::List(list) => Some(&list.args),
        _ => None,
    }
}

fn as_symbol(e: &ExprKind) -> Option<String> {
    match e {
        ExprKind::Atom(a) => match &a.syn.ty {
            TokenType::Identifier(s) => Some(s.resolve().to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn as_string(e: &ExprKind) -> Option<String> {
    match e {
        ExprKind::Atom(a) => match &a.syn.ty {
            TokenType::StringLiteral(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn as_keyword(e: &ExprKind) -> Option<String> {
    match e {
        ExprKind::Atom(a) => match &a.syn.ty {
            TokenType::Keyword(s) => Some(s.resolve().trim_start_matches("#:").to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// The verbatim source slice covered by a datum's span.
fn span_src<'a>(e: &ExprKind, src: &'a str) -> Option<&'a str> {
    let span = e.span()?;
    src.get(span.start as usize..span.end as usize)
}

fn int_field(e: &ExprKind, src: &str) -> Result<i64, FormatError> {
    span_src(e, src)
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("expected an integer".into())))
}

fn float_field(e: &ExprKind, src: &str) -> Result<f32, FormatError> {
    span_src(e, src)
        .and_then(|s| s.parse::<f32>().ok())
        .ok_or_else(|| err_at(e, src, ErrorKind::Malformed("expected a number".into())))
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

fn err_at(e: &ExprKind, src: &str, kind: ErrorKind) -> FormatError {
    let span = e
        .span()
        .map(|s| Span::new(s.start as usize, s.end as usize))
        .unwrap_or_default();
    FormatError::new(kind).at(span, src)
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
        let file = parse_file(text).expect("parse");
        assert_eq!(file.graphs.len(), 1);
        let g = &file.graphs[0];
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
    fn reads_keywords_branch_and_ref() {
        let text = "\
(graph g
  (s (expr (values $x (* $x 2)) #:out 2))
  (b (branch (if $n (list 0 0) (list 1 0)) \"10\" \"01\"))
  (m (ref mul \"834568e9\")))";
        let file = parse_file(text).expect("parse");
        let g = &file.graphs[0];
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
