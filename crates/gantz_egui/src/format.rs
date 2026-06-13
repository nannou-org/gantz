//! The Export-level `.gantz` format.
//!
//! [`gantz_format`] owns the layout-agnostic registry format. This module
//! layers GUI view state on top - `(layout ...)` and `(demo ...)` forms - using
//! the format's [`sexpr`] toolkit and the resolution
//! context returned by [`gantz_format::from_str`]/[`gantz_format::to_string`].
//! It produces and consumes a full [`Export`] (registry + views + demos), so
//! existing `crate::format::{from_str, to_string}` call sites are unchanged.

use crate::GraphViews;
use crate::export::Export;
use gantz_ca::{CommitAddr, Registry, Timestamp};
use gantz_core::node::graph::Graph;
use gantz_format::sexpr;
use gantz_format::{Addr, Form, GraphLabels, Loaded};
use std::collections::HashMap;

pub use gantz_format::{FormatError, Lowerable};

/// Parse a `.gantz` document into an [`Export`] (registry + views + demos).
///
/// `now` provides the timestamp for any graph the document does not commit
/// explicitly (hand-authored graphs).
pub fn from_str<N>(text: &str, now: Timestamp) -> Result<Export<Graph<N>>, FormatError>
where
    N: Lowerable,
{
    let loaded = gantz_format::from_str::<N>(text, now)?;
    let mut views: HashMap<CommitAddr, GraphViews> = HashMap::new();
    let mut demos: HashMap<CommitAddr, String> = HashMap::new();
    for form in &loaded.extra {
        match form.head.as_str() {
            "layout" => apply_layout(form, &loaded, &mut views),
            "demo" => apply_demo(form, &loaded, &mut demos),
            other => log::warn!("ignoring unrecognised `.gantz` form `{other}`"),
        }
    }
    Ok(Export {
        registry: loaded.registry,
        views,
        demos,
    })
}

/// Serialize an [`Export`] to a `.gantz` document.
pub fn to_string<N>(export: &Export<Graph<N>>) -> Result<String, FormatError>
where
    N: Lowerable,
{
    let dumped = gantz_format::to_string::<N>(&export.registry)?;
    // Each top-level block is a section; they are joined with a blank line.
    let mut sections = vec![dumped.text.trim_end().to_string()];

    // `(layout ...)` per graph that has a top-level view, keyed by graph id.
    let mut views: Vec<_> = export.views.iter().collect();
    views.sort_by_key(|(ca, _)| **ca);
    for (commit_ca, gv) in views {
        let (Some(view), Some(commit)) = (
            gv.get(&Vec::new()),
            export.registry.commits().get(commit_ca),
        ) else {
            continue;
        };
        if let Some(labels) = dumped.graphs.get(&commit.graph) {
            sections.push(layout_text(labels, view));
        }
    }

    // `(demo <name> ...)` per commit that has a demo.
    let mut demos: Vec<_> = export.demos.iter().collect();
    demos.sort_by_key(|(ca, _)| **ca);
    for (commit_ca, demo) in demos {
        if let Some(name) = name_for_commit(&export.registry, commit_ca) {
            sections.push(format!("(demo {} {})", name, sexpr::quote(demo)));
        }
    }

    let mut result = sections.join("\n\n");
    result.push('\n');
    Ok(result)
}

// -- layout ------------------------------------------------------------------

fn apply_layout<N>(form: &Form, loaded: &Loaded<N>, views: &mut HashMap<CommitAddr, GraphViews>) {
    let src = &form.raw;
    let Ok(forms) = sexpr::read(src) else { return };
    let Some(args) = forms.first().and_then(sexpr::list_args) else {
        return;
    };
    // args = [layout, <graph-id>, <entry>...]
    let Some(graph_id) = args.get(1).and_then(addr_of) else {
        return;
    };
    let (Some(&head), Some(index)) = (
        loaded.graph_head.get(&graph_id),
        loaded.index.get(&graph_id),
    ) else {
        return;
    };

    let mut layout = egui_graph::Layout::default();
    let mut scene = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(0.0, 0.0));
    for entry in &args[2..] {
        let Some(eargs) = sexpr::list_args(entry) else {
            continue;
        };
        let Some(head_sym) = eargs.first().and_then(sexpr::as_symbol) else {
            continue;
        };
        if head_sym == "scene" {
            let f: Vec<f32> = eargs[1..]
                .iter()
                .filter_map(|n| sexpr::as_f32(n, src))
                .collect();
            if f.len() == 4 {
                scene = egui::Rect::from_min_max(egui::pos2(f[0], f[1]), egui::pos2(f[2], f[3]));
            }
        } else if let (Some(x), Some(y)) = (
            eargs.get(1).and_then(|n| sexpr::as_f32(n, src)),
            eargs.get(2).and_then(|n| sexpr::as_f32(n, src)),
        ) {
            if let Some(&ix) = index.get(&head_sym) {
                layout.insert(egui_graph::NodeId(ix as u64), egui::pos2(x, y));
            }
        }
    }

    let view = egui_graph::View {
        scene_rect: scene,
        layout,
    };
    views.entry(head).or_default().insert(Vec::new(), view);
}

fn layout_text(labels: &GraphLabels, view: &egui_graph::View) -> String {
    let mut positions: Vec<(String, f32, f32)> = view
        .layout
        .iter()
        .filter_map(|(nid, pos)| {
            labels
                .labels
                .get(&(nid.0 as usize))
                .map(|l| (l.clone(), pos.x, pos.y))
        })
        .collect();
    positions.sort_by(|a, b| a.0.cmp(&b.0));

    let mut s = format!("(layout {}", sexpr::quote(&labels.id));
    for (label, x, y) in positions {
        s.push_str(&format!(
            "\n  ({label} {} {})",
            sexpr::num(x),
            sexpr::num(y)
        ));
    }
    let r = view.scene_rect;
    s.push_str(&format!(
        "\n  (scene {} {} {} {}))",
        sexpr::num(r.min.x),
        sexpr::num(r.min.y),
        sexpr::num(r.max.x),
        sexpr::num(r.max.y),
    ));
    s
}

// -- demos -------------------------------------------------------------------

fn apply_demo<N>(form: &Form, loaded: &Loaded<N>, demos: &mut HashMap<CommitAddr, String>) {
    let src = &form.raw;
    let Ok(forms) = sexpr::read(src) else { return };
    let Some(args) = forms.first().and_then(sexpr::list_args) else {
        return;
    };
    // args = [demo, <name>, "<demo>"]
    let (Some(name), Some(demo)) = (
        args.get(1).and_then(sexpr::as_symbol),
        args.get(2).and_then(sexpr::as_string),
    ) else {
        return;
    };
    if let Some(&commit) = loaded.names.get(&name) {
        demos.insert(commit, demo);
    }
}

// -- helpers -----------------------------------------------------------------

/// Read an [`Addr`] from a datum: a string is concrete, a symbol is a label.
fn addr_of(e: &sexpr::ExprKind) -> Option<Addr> {
    sexpr::as_string(e)
        .map(Addr::Concrete)
        .or_else(|| sexpr::as_symbol(e).map(Addr::Label))
}

/// The first registry name pointing at `commit`, if any.
fn name_for_commit<N>(registry: &Registry<Graph<N>>, commit: &CommitAddr) -> Option<String> {
    registry
        .names()
        .iter()
        .find(|(_, ca)| *ca == commit)
        .map(|(name, _)| name.clone())
}
