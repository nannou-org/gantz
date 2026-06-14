//! Serializes a [`Document`] to `.gantz` text.
//!
//! Output is reader-valid Steel: node code is spliced verbatim (it is already
//! valid Steel), addresses are strings, placeholders are symbols, and node
//! ports are `(name port)` sub-lists. Indentation is two spaces per nesting
//! level.

use crate::datum::{Datum, datum_text};
use crate::model::{
    Addr, CommitDecl, Conn, Document, Endpoint, GraphBody, NameDecl, NodeDecl, NodeSpec,
};
use crate::sexpr::quote;
use crate::sugar::Sugar;

/// Serialize a [`Document`]'s registry forms to `.gantz` text, rendering node
/// sugar with `sugar`.
pub fn write_document(doc: &Document, sugar: &dyn Sugar) -> String {
    let mut out = String::new();
    for def in &doc.graphs {
        write_graph(&mut out, &def.id, &def.body, sugar);
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

fn write_graph(out: &mut String, id: &Addr, body: &GraphBody, sugar: &dyn Sugar) {
    write_graph_body(out, &format!("graph {}", addr_text(id)), body, 0, sugar);
}

/// Write `(<head> <nodes...> <conns...>)` with the body indented one level
/// deeper than `indent`.
fn write_graph_body(
    out: &mut String,
    head: &str,
    body: &GraphBody,
    indent: usize,
    sugar: &dyn Sugar,
) {
    out.push_str(&format!("({head}"));
    let inner = indent + 1;
    let pad = "  ".repeat(inner);
    for decl in &body.nodes {
        out.push('\n');
        out.push_str(&pad);
        write_node_decl(out, decl, sugar);
    }
    for conn in &body.conns {
        out.push('\n');
        out.push_str(&pad);
        out.push_str(&conn_text(conn));
    }
    out.push(')');
}

fn write_node_decl(out: &mut String, decl: &NodeDecl, sugar: &dyn Sugar) {
    match &decl.spec {
        NodeSpec::Value(v) => out.push_str(&format!("({} {})", decl.name, value_spec(v, sugar))),
        NodeSpec::Ref(r) => out.push_str(&format!("({} {})", decl.name, ref_spec(r))),
    }
}

/// Render a node value: a sugared form if `sugar` provides one, else the generic
/// `(node "Tag" ...)` fallback.
fn value_spec(v: &Datum, sugar: &dyn Sugar) -> String {
    let tag = v.get("type").and_then(Datum::as_str).unwrap_or("node");
    sugar.write_spec(tag, v).unwrap_or_else(|| generic_spec(v))
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

fn generic_spec(v: &Datum) -> String {
    let tag = v.get("type").and_then(Datum::as_str).unwrap_or("node");
    let mut s = format!("(node {}", quote(tag));
    if let Datum::Map(entries) = v {
        for (k, val) in entries {
            if k == "type" {
                continue;
            }
            s.push_str(&format!(" ({k} {})", datum_text(val)));
        }
    }
    s.push(')');
    s
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
