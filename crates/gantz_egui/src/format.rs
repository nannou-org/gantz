//! The Export-level `.gantz` format.
//!
//! [`gantz_format`] owns the layout-agnostic registry format. This module
//! layers GUI view state on top - `(layout ...)` and `(demo ...)` forms - using
//! the format's [`sexpr`] toolkit and the resolution
//! context returned by [`gantz_format::from_str`]/[`gantz_format::to_string`].
//! It produces and consumes a full [`Export`] (registry + views + demos), so
//! existing `crate::format::{from_str, to_string}` call sites are unchanged.

use crate::export::Export;
use gantz_ca::{CaHash, CommitAddr, Timestamp};
use gantz_core::node::graph::Graph;
use gantz_format::sexpr;
use gantz_format::{Addr, Form, GraphLabels, Loaded};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;

pub use gantz_format::FormatError;

/// Parse a `.gantz` document into an [`Export`] (registry + views + demos).
///
/// `now` provides the timestamp for any graph the document does not commit
/// explicitly (hand-authored graphs).
pub fn from_str<N>(text: &str, now: Timestamp) -> Result<Export<Graph<N>>, FormatError>
where
    N: Serialize + DeserializeOwned + CaHash + 'static,
{
    let loaded = gantz_format::from_str::<N>(text, now)?;
    let mut views: HashMap<CommitAddr, crate::SceneView> = HashMap::new();
    let mut demos: HashMap<String, String> = HashMap::new();
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
    N: Serialize + DeserializeOwned,
{
    let dumped = gantz_format::to_string::<N>(&export.registry)?;
    // Each top-level block is a section; they are joined with a blank line.
    let mut sections = vec![dumped.text.trim_end().to_string()];

    // `(layout ...)` per graph that has a top-level view, keyed by graph id.
    let mut views: Vec<_> = export.views.iter().collect();
    views.sort_by_key(|(ca, _)| **ca);
    for (commit_ca, view) in views {
        let Some(commit) = export.registry.commits().get(commit_ca) else {
            continue;
        };
        if let Some(labels) = dumped.graphs.get(&commit.graph) {
            sections.push(layout_text(labels, view, false));
        }
    }

    // `(demo <name> ...)` per named graph that has a demo, in name order.
    let mut demos: Vec<_> = export.demos.iter().collect();
    demos.sort_by(|a, b| a.0.cmp(b.0));
    for (name, demo) in demos {
        sections.push(format!("(demo {} {})", name, sexpr::quote(demo)));
    }

    let mut result = sections.join("\n\n");
    result.push('\n');
    Ok(result)
}

/// Serialize an [`Export`] in the inline-name format (see
/// [`gantz_format::to_string_named`]): graphs named inline, no commits/names
/// tables, references by name. The `(layout ...)` and `(demo ...)` forms are
/// emitted in graph-name order so the output is stable across address changes -
/// suited to a hand-editable, git-friendly base file.
pub fn to_string_named<N>(export: &Export<Graph<N>>) -> Result<String, FormatError>
where
    N: Serialize + DeserializeOwned,
{
    let dumped = gantz_format::to_string_named::<N>(&export.registry)?;
    let mut sections = vec![dumped.text.trim_end().to_string()];

    // `(layout ...)` per named graph that has a view, in name order.
    for (_name, commit_ca) in export.registry.names() {
        let (Some(view), Some(commit)) = (
            export.views.get(commit_ca),
            export.registry.commits().get(commit_ca),
        ) else {
            continue;
        };
        if let Some(labels) = dumped.graphs.get(&commit.graph) {
            sections.push(layout_text(labels, view, true));
        }
    }

    // `(demo <name> ...)` per named graph that has a demo, in name order.
    for (name, _commit_ca) in export.registry.names() {
        if let Some(demo) = export.demos.get(name) {
            sections.push(format!("(demo {} {})", name, sexpr::quote(demo)));
        }
    }

    let mut result = sections.join("\n\n");
    result.push('\n');
    Ok(result)
}

// -- layout ------------------------------------------------------------------

fn apply_layout<N>(
    form: &Form,
    loaded: &Loaded<N>,
    views: &mut HashMap<CommitAddr, crate::SceneView>,
) {
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
    let mut camera = crate::Camera::default();
    for entry in &args[2..] {
        let Some(eargs) = sexpr::list_args(entry) else {
            continue;
        };
        let Some(head_sym) = eargs.first().and_then(sexpr::as_symbol) else {
            continue;
        };
        if head_sym == "camera" {
            let f: Vec<f32> = eargs[1..]
                .iter()
                .filter_map(|n| sexpr::as_f32(n, src))
                .collect();
            if f.len() == 3 {
                camera = crate::Camera {
                    center: egui::pos2(f[0], f[1]),
                    zoom: f[2],
                };
            }
        } else if head_sym == "scene" {
            // Legacy: a visible-region rect (pre-camera format). Recover the
            // centre at the default zoom; the exact zoom can't be reconstructed
            // without the viewport the rect was captured against.
            let f: Vec<f32> = eargs[1..]
                .iter()
                .filter_map(|n| sexpr::as_f32(n, src))
                .collect();
            if f.len() == 4 {
                let rect = egui::Rect::from_min_max(egui::pos2(f[0], f[1]), egui::pos2(f[2], f[3]));
                camera = crate::Camera {
                    center: rect.center(),
                    zoom: 1.0,
                };
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

    let view = crate::SceneView { camera, layout };
    views.insert(head, view);
}

/// `bare_id` writes the graph id as a bare symbol (the inline-name format, where
/// the graph itself is `(graph <name> ...)`); otherwise it is quoted (the
/// address-based format, `(graph "<hex>" ...)`). The id must round-trip to the
/// same `Addr` kind as the graph, or the layout fails to resolve on load.
fn layout_text(labels: &GraphLabels, view: &crate::SceneView, bare_id: bool) -> String {
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

    let id = if bare_id {
        labels.id.clone()
    } else {
        sexpr::quote(&labels.id)
    };
    let mut s = format!("(layout {id}");
    for (label, x, y) in positions {
        s.push_str(&format!(
            "\n  ({label} {} {})",
            sexpr::num(x),
            sexpr::num(y)
        ));
    }
    let c = view.camera;
    s.push_str(&format!(
        "\n  (camera {} {} {}))",
        sexpr::num(c.center.x),
        sexpr::num(c.center.y),
        sexpr::num(c.zoom),
    ));
    s
}

// -- demos -------------------------------------------------------------------

fn apply_demo<N>(form: &Form, loaded: &Loaded<N>, demos: &mut HashMap<String, String>) {
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
    // Only retain demos for names this document's registry actually defines.
    if loaded.names.contains_key(&name) {
        demos.insert(name, demo);
    }
}

// -- helpers -----------------------------------------------------------------

/// Read an [`Addr`] from a datum: a string is concrete, a symbol is a label.
fn addr_of(e: &sexpr::ExprKind) -> Option<Addr> {
    sexpr::as_string(e)
        .map(Addr::Concrete)
        .or_else(|| sexpr::as_symbol(e).map(Addr::Label))
}
