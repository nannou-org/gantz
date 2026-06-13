//! Serializes a [`Document`] to `.gantz` text.
//!
//! Output is reader-valid Steel: node code is spliced verbatim (it is already
//! valid Steel), addresses are strings, placeholders are symbols, and node
//! ports are `(name port)` sub-lists. Indentation is two spaces per nesting
//! level.

use crate::model::{
    Addr, CommitDecl, Conn, Document, Endpoint, GraphBody, NameDecl, NodeDecl, NodeSpec,
};
use crate::sexpr::quote;
use crate::sugar::keyword_for_tag;
use serde_json::Value;

/// Serialize a [`Document`]'s registry forms to `.gantz` text.
pub fn write_document(doc: &Document) -> String {
    let mut out = String::new();
    for def in &doc.graphs {
        write_graph(&mut out, &def.id, &def.body);
        out.push_str("\n\n");
    }
    if !doc.commits.is_empty() {
        write_commits(&mut out, &doc.commits);
        out.push_str("\n\n");
    }
    if !doc.names.is_empty() {
        write_names(&mut out, &doc.names);
        out.push_str("\n\n");
    }
    let trimmed = out.trim_end();
    let mut result = trimmed.to_string();
    result.push('\n');
    result
}

// -- graphs ------------------------------------------------------------------

fn write_graph(out: &mut String, id: &Addr, body: &GraphBody) {
    write_graph_body(out, &format!("graph {}", addr_text(id)), body, 0);
}

/// Write `(<head> <nodes...> <conns...>)` with the body indented one level
/// deeper than `indent`.
fn write_graph_body(out: &mut String, head: &str, body: &GraphBody, indent: usize) {
    out.push_str(&format!("({head}"));
    let inner = indent + 1;
    let pad = "  ".repeat(inner);
    for decl in &body.nodes {
        out.push('\n');
        out.push_str(&pad);
        write_node_decl(out, decl, inner);
    }
    for conn in &body.conns {
        out.push('\n');
        out.push_str(&pad);
        out.push_str(&conn_text(conn));
    }
    out.push(')');
}

fn write_node_decl(out: &mut String, decl: &NodeDecl, indent: usize) {
    match &decl.spec {
        NodeSpec::Graph(body) => {
            out.push_str(&format!("({} ", decl.name));
            write_graph_body(out, "graph", body, indent);
            out.push(')');
        }
        NodeSpec::Value(v) => out.push_str(&format!("({} {})", decl.name, value_spec(v))),
        NodeSpec::Ref(r) => out.push_str(&format!("({} {})", decl.name, ref_spec(r))),
    }
}

fn value_spec(v: &Value) -> String {
    let tag = v.get("type").and_then(Value::as_str).unwrap_or("node");
    match tag {
        "Expr" => {
            let src = v.get("src").and_then(Value::as_str).unwrap_or("'()");
            match v.get("outputs").and_then(Value::as_u64) {
                Some(n) if n != 1 => format!("(expr {src} #:out {n})"),
                _ => format!("(expr {src})"),
            }
        }
        "Branch" => {
            let src = v.get("src").and_then(Value::as_str).unwrap_or("'()");
            let masks = v
                .get("branches")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(quote)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            format!("(branch {src} {masks})")
        }
        "Comment" => {
            let text = v.get("text").and_then(Value::as_str).unwrap_or("");
            let (w, h) = v
                .get("size")
                .and_then(Value::as_array)
                .and_then(|a| Some((a.first()?.as_u64()?, a.get(1)?.as_u64()?)))
                .unwrap_or((100, 40));
            format!("(comment {} {w} {h})", quote(text))
        }
        "Log" => match v.get("level").and_then(Value::as_str) {
            Some(level) if !level.eq_ignore_ascii_case("info") => {
                format!("(log {})", level.to_ascii_lowercase())
            }
            _ => "(log)".to_string(),
        },
        other => match keyword_for_tag(other) {
            Some(keyword) => keyword.to_string(),
            None => generic_spec(v),
        },
    }
}

fn ref_spec(r: &crate::model::RefSpec) -> String {
    let keyword = if r.func { "fn-ref" } else { "ref" };
    let mut s = format!("({keyword} {}", r.name);
    if let Some(addr) = &r.addr {
        s.push(' ');
        s.push_str(&addr_text(addr));
    }
    if r.sync {
        s.push_str(" #:sync");
    }
    s.push(')');
    s
}

fn generic_spec(v: &Value) -> String {
    let tag = v.get("type").and_then(Value::as_str).unwrap_or("node");
    let mut s = format!("(node {}", quote(tag));
    if let Value::Object(map) = v {
        for (k, val) in map {
            if k == "type" {
                continue;
            }
            s.push_str(&format!(" ({k} {})", datum_text(val)));
        }
    }
    s.push(')');
    s
}

/// Render a JSON value as a reader-valid datum.
fn datum_text(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => quote(s),
        Value::Array(items) => {
            let inner = items.iter().map(datum_text).collect::<Vec<_>>().join(" ");
            format!("({inner})")
        }
        Value::Object(map) => {
            let inner = map
                .iter()
                .map(|(k, val)| format!("({k} {})", datum_text(val)))
                .collect::<Vec<_>>()
                .join(" ");
            format!("({inner})")
        }
    }
}

// -- connections -------------------------------------------------------------

fn conn_text(conn: &Conn) -> String {
    format!(
        "(-> {} {})",
        endpoint_text(&conn.from),
        endpoint_text(&conn.to)
    )
}

fn endpoint_text(ep: &Endpoint) -> String {
    if ep.port == 0 {
        ep.node.clone()
    } else {
        format!("({} {})", ep.node, ep.port)
    }
}

// -- commits / names ---------------------------------------------------------

fn write_commits(out: &mut String, commits: &[CommitDecl]) {
    out.push_str("(commits");
    for c in commits {
        out.push_str(&format!("\n  {}", commit_text(c)));
    }
    out.push(')');
}

fn commit_text(c: &CommitDecl) -> String {
    let mut s = format!("({} (time {} {})", addr_text(&c.id), c.secs, c.nanos);
    if let Some(addr) = &c.parent {
        s.push_str(&format!(" (parent {})", addr_text(addr)));
    }
    s.push_str(&format!(" (graph {}))", addr_text(&c.graph)));
    s
}

fn write_names(out: &mut String, names: &[NameDecl]) {
    out.push_str("(names");
    for n in names {
        out.push_str(&format!("\n  ({} {})", n.name, addr_text(&n.commit)));
    }
    out.push(')');
}

// -- helpers -----------------------------------------------------------------

fn addr_text(addr: &Addr) -> String {
    match addr {
        Addr::Concrete(hex) => quote(hex),
        Addr::Label(label) => label.clone(),
    }
}
