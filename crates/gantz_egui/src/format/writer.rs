//! Serializes a [`File`] AST to `.gantz` text.
//!
//! Output is reader-valid Steel: node code is spliced verbatim (it is already
//! valid Steel), addresses are strings, placeholders are symbols, and node
//! ports are `(name port)` sub-lists. Indentation is two spaces per nesting
//! level.

use super::model::{
    Addr, CommitDecl, Conn, Endpoint, File, GraphBody, GraphId, History, Layout, NodeDecl, NodeSpec,
};
use super::sugar::keyword_for_tag;
use serde_json::Value;

/// Serialize a [`File`] AST to a `.gantz` document.
pub fn write_file(file: &File) -> String {
    let mut out = String::new();
    for def in &file.graphs {
        write_graph(&mut out, &def.id, &def.body);
        out.push_str("\n\n");
    }
    for layout in &file.layouts {
        write_layout(&mut out, layout);
        out.push_str("\n\n");
    }
    for history in &file.histories {
        write_history(&mut out, history);
        out.push_str("\n\n");
    }
    for demo in &file.demos {
        out.push_str(&format!("(demo {} {})\n\n", demo.graph, quote(&demo.demo)));
    }
    let trimmed = out.trim_end();
    let mut result = trimmed.to_string();
    result.push('\n');
    result
}

// -- graphs ------------------------------------------------------------------

fn write_graph(out: &mut String, id: &GraphId, body: &GraphBody) {
    let head = match id {
        GraphId::Name(name) => format!("graph {name}"),
        GraphId::Addr(hex) => format!("graph {}", quote(hex)),
    };
    write_graph_body(out, &head, body, 0);
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

fn ref_spec(r: &super::model::RefSpec) -> String {
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

// -- layout ------------------------------------------------------------------

fn write_layout(out: &mut String, layout: &Layout) {
    out.push_str(&format!("(layout {}", layout.graph));
    if !layout.path.is_empty() {
        out.push_str(&format!("\n  (at {})", layout.path.join(" ")));
    }
    for (name, x, y) in &layout.positions {
        out.push_str(&format!("\n  ({name} {} {})", num(*x), num(*y)));
    }
    if let Some([min_x, min_y, max_x, max_y]) = layout.scene {
        out.push_str(&format!(
            "\n  (scene {} {} {} {})",
            num(min_x),
            num(min_y),
            num(max_x),
            num(max_y)
        ));
    }
    out.push(')');
}

// -- history -----------------------------------------------------------------

fn write_history(out: &mut String, history: &History) {
    out.push_str(&format!("(history {}", history.graph));
    for commit in &history.commits {
        out.push_str(&format!("\n  {}", commit_text(commit)));
    }
    out.push(')');
}

fn commit_text(c: &CommitDecl) -> String {
    let mut s = format!("(commit {} (time {} {})", addr_text(&c.id), c.secs, c.nanos);
    match &c.parent {
        Some(addr) => s.push_str(&format!(" (parent {})", addr_text(addr))),
        None => s.push_str(" (parent none)"),
    }
    if let Some(addr) = &c.graph {
        s.push_str(&format!(" (graph {})", addr_text(addr)));
    }
    s.push(')');
    s
}

// -- helpers -----------------------------------------------------------------

fn addr_text(addr: &Addr) -> String {
    match addr {
        Addr::Concrete(hex) => quote(hex),
        Addr::Label(label) => label.clone(),
    }
}

/// Format a float without scientific notation, using the shortest round-tripping
/// representation.
fn num(x: f32) -> String {
    let s = format!("{x}");
    if s.contains('e') || s.contains('E') {
        format!("{x:.6}")
    } else {
        s
    }
}

/// Quote a string as a Steel string literal.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}
