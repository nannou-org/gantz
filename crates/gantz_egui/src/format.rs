//! The GUI layer of the `.gantz` format.
//!
//! [`gantz_format`] owns the layout-agnostic registry format. This module
//! renders the GUI's registry sections (see [`crate::section`]) as friendly
//! forms - `(descriptions ...)`, `(layout ...)` and `(demo ...)` - using the
//! format's [`sexpr`] toolkit and the resolution context returned by
//! [`gantz_format::from_str`]/[`gantz_format::to_string`], and applies those
//! forms back into the registry's sections on parse.

use crate::node::NodeCodec;
use gantz_ca::{Datum, GraphAddr, Name, NodeData, Registry, Timestamp};
use gantz_format::sexpr;
use gantz_format::{Addr, Form, GraphLabels, Loaded};

pub use gantz_format::FormatError;

/// The section ids this module renders itself as friendly forms (passed as
/// `claimed` to [`gantz_format::to_string`], which then skips their generic
/// `(section ...)` output).
const CLAIMED: &[&str] = &[
    crate::section::DESCRIPTIONS_ID,
    crate::section::VIEWS_ID,
    crate::section::DEMOS_ID,
];

/// The [`gantz_format::Normalize`] implementation backed by a [`NodeCodec`]:
/// split the parsed datum's `"type"` tag out and round-trip the fields
/// through the codec's typed node ([`NodeCodec::normalize`]), recomputing the
/// canonical form and the refs/blobs columns.
fn normalize_datum(codec: &NodeCodec, datum: Datum) -> Result<NodeData, FormatError> {
    let Datum::Map(mut entries) = datum else {
        return Err(FormatError::malformed("node datum is not a map"));
    };
    let Some(ix) = entries.iter().position(|(k, _)| k == "type") else {
        return Err(FormatError::malformed("node datum has no `type` tag"));
    };
    let (_, tag) = entries.remove(ix);
    let Datum::Str(tag) = tag else {
        return Err(FormatError::malformed("node `type` tag is not a string"));
    };
    codec
        .normalize(&NodeData::new(tag.clone(), Datum::Map(entries)))
        .map_err(|e| FormatError::node_deserialize(tag, e.to_string()))
}

/// Parse a `.gantz` document into a registry, applying the GUI-layer friendly
/// forms (`descriptions`, `layout`, `demo`) into its sections.
///
/// `now` provides the timestamp for any graph the document does not commit
/// explicitly (hand-authored graphs).
pub fn from_str(text: &str, now: Timestamp, codec: &NodeCodec) -> Result<Registry, FormatError> {
    from_str_seeded(text, now, &std::collections::BTreeMap::new(), codec)
}

/// [`from_str`], resolving names the document does not define through `seed`
/// (externally-known name -> head graph associations). See
/// [`gantz_format::from_str_seeded`].
pub fn from_str_seeded(
    text: &str,
    now: Timestamp,
    seed: &std::collections::BTreeMap<String, GraphAddr>,
    codec: &NodeCodec,
) -> Result<Registry, FormatError> {
    let loaded = gantz_format::from_str_normalized(text, now, &codec.sugars(), seed, &|datum| {
        normalize_datum(codec, datum)
    })?;
    Ok(registry_from_loaded(loaded))
}

/// Apply the GUI-layer extra forms (`descriptions`, `layout`, `demo`) to a
/// loaded registry's sections.
fn registry_from_loaded(mut loaded: Loaded) -> Registry {
    let extra = std::mem::take(&mut loaded.extra);
    for form in &extra {
        match form.head.as_str() {
            "descriptions" => apply_descriptions(form, &mut loaded),
            "layout" => apply_layout(form, &mut loaded),
            "demo" => apply_demo(form, &mut loaded),
            other => log::warn!("ignoring unrecognised `.gantz` form `{other}`"),
        }
    }
    loaded.registry
}

/// Serialize a registry to a `.gantz` document, with the codec supplying the
/// node set's keyword [`gantz_format::Sugar`] for the graph forms.
pub fn to_string(registry: &Registry, codec: &NodeCodec) -> Result<String, FormatError> {
    let dumped = gantz_format::to_string_with(registry, &codec.sugars(), CLAIMED)?;
    // Each top-level block is a section; they are joined with a blank line.
    let mut sections = vec![dumped.text.trim_end().to_string()];

    // `(descriptions ...)`, in name order.
    sections.extend(descriptions_text(registry));

    // `(layout ...)` per commit that has a stored view, keyed by graph id.
    for (commit_ca, view) in crate::section::views(registry) {
        let Some(commit) = registry.commits().get(&commit_ca) else {
            continue;
        };
        if let Some(labels) = dumped.graphs.get(&commit.graph) {
            sections.push(layout_text(labels, &view, false));
        }
    }

    // `(demo <name> ...)` per named graph that has a demo, in name order.
    for (name, demo) in crate::section::demos(registry) {
        sections.push(format!("(demo {} {})", name, sexpr::quote(&demo)));
    }

    let mut result = sections.join("\n\n");
    result.push('\n');
    Ok(result)
}

/// Serialize a registry in the inline-name format (see
/// [`gantz_format::to_string_named`]): graphs named inline, no commits/names
/// tables, references by name. The `(layout ...)` and `(demo ...)` forms are
/// emitted in graph-name order so the output is stable across address changes -
/// suited to a hand-editable, git-friendly base file.
pub fn to_string_named(registry: &Registry, codec: &NodeCodec) -> Result<String, FormatError> {
    let dumped = gantz_format::to_string_named_with(registry, &codec.sugars(), CLAIMED)?;
    let mut sections = vec![dumped.text.trim_end().to_string()];

    // `(descriptions ...)`, in name order.
    sections.extend(descriptions_text(registry));

    // `(layout ...)` per named graph that has a view, in name order.
    for (_name, commit_ca) in registry.heads() {
        let (Some(view), Some(commit)) = (
            crate::section::view(registry, &commit_ca),
            registry.commits().get(&commit_ca),
        ) else {
            continue;
        };
        if let Some(labels) = dumped.graphs.get(&commit.graph) {
            sections.push(layout_text(labels, &view, true));
        }
    }

    // `(demo <name> ...)` per named graph that has a demo, in name order.
    for (name, demo) in crate::section::demos(registry) {
        sections.push(format!("(demo {} {})", name, sexpr::quote(&demo)));
    }

    let mut result = sections.join("\n\n");
    result.push('\n');
    Ok(result)
}

// -- descriptions -------------------------------------------------------------

fn apply_descriptions(form: &Form, loaded: &mut Loaded) {
    let src = &form.raw;
    let Ok(forms) = sexpr::read(src) else { return };
    let Some(args) = forms.first().and_then(sexpr::list_args) else {
        return;
    };
    // args = [descriptions, (<name> "<text>")...]
    for entry in &args[1..] {
        let Some(eargs) = sexpr::list_args(entry) else {
            continue;
        };
        let (Some(name), Some(text)) = (
            eargs.first().and_then(sexpr::as_symbol),
            eargs.get(1).and_then(sexpr::as_string),
        ) else {
            continue;
        };
        // Only retain descriptions for names this document actually defines.
        if loaded.names.contains_key(&name) {
            let name: Name = name.parse().expect("infallible");
            crate::section::set_description(&mut loaded.registry, name, text);
        }
    }
}

/// The `(descriptions ...)` form for the registry's description section, in
/// name order. `None` when there are no descriptions.
fn descriptions_text(registry: &Registry) -> Option<String> {
    let mut entries = crate::section::descriptions(registry).peekable();
    entries.peek()?;
    let mut s = "(descriptions".to_string();
    for (name, text) in entries {
        s.push_str(&format!("\n  ({} {})", name, sexpr::quote(&text)));
    }
    s.push(')');
    Some(s)
}

// -- layout --------------------------------------------------------------------

fn apply_layout(form: &Form, loaded: &mut Loaded) {
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
    crate::section::set_view(&mut loaded.registry, head, &view);
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

fn apply_demo(form: &Form, loaded: &mut Loaded) {
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
        let name: Name = name.parse().expect("infallible");
        crate::section::set_demo(&mut loaded.registry, name, demo);
    }
}

// -- helpers -----------------------------------------------------------------

/// Read an [`Addr`] from a datum: a string is concrete, a symbol is a label.
fn addr_of(e: &sexpr::ExprKind) -> Option<Addr> {
    sexpr::as_string(e)
        .map(Addr::Concrete)
        .or_else(|| sexpr::as_symbol(e).map(Addr::Label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_node::{TestGraph, codec, commit_named, expr, named_ref};
    use gantz_ca::CommitAddr;
    use std::time::Duration;

    fn name(s: &str) -> Name {
        s.parse().unwrap()
    }

    /// A registry with a `leaf` expr graph, a `root` graph referencing it,
    /// and a description, demo and view attached to `root`.
    fn test_registry() -> (Registry, CommitAddr) {
        let mut reg = Registry::default();

        let mut leaf_g = TestGraph::default();
        leaf_g.add_node(expr("(+ 1 1)"));
        let (_, leaf_ga) = commit_named(&mut reg, Duration::from_secs(1), &leaf_g, &name("leaf"));

        let mut root_g = TestGraph::default();
        root_g.add_node(named_ref("leaf", leaf_ga));
        let (root_ca, _) = commit_named(&mut reg, Duration::from_secs(2), &root_g, &name("root"));

        crate::section::set_description(&mut reg, name("root"), "the root".to_string());
        crate::section::set_demo(&mut reg, name("root"), "demo-root".to_string());
        let view = crate::SceneView {
            camera: crate::Camera {
                center: egui::pos2(3.0, 4.0),
                zoom: 2.0,
            },
            layout: [(egui_graph::NodeId(0), egui::pos2(10.0, 20.0))]
                .into_iter()
                .collect(),
        };
        crate::section::set_view(&mut reg, root_ca, &view);

        (reg, root_ca)
    }

    fn assert_sections_survive(parsed: &Registry) {
        assert!(parsed.head(&name("leaf")).is_some());
        let root_ca = parsed.head(&name("root")).expect("root head survives");
        assert_eq!(
            crate::section::description(parsed, &name("root")).as_deref(),
            Some("the root"),
        );
        assert_eq!(
            crate::section::demo(parsed, &name("root")).as_deref(),
            Some("demo-root"),
        );
        let view = crate::section::view(parsed, &root_ca).expect("view survives");
        assert_eq!(
            view.layout.get(&egui_graph::NodeId(0)).copied(),
            Some(egui::pos2(10.0, 20.0)),
        );
        assert_eq!(view.camera.center, egui::pos2(3.0, 4.0));
        assert_eq!(view.camera.zoom, 2.0);
    }

    /// Descriptions, demos and views survive an address-based text
    /// round-trip via their friendly forms.
    #[test]
    fn sections_round_trip_through_text() {
        let (reg, root_ca) = test_registry();
        let text = to_string(&reg, &codec()).unwrap();
        // Claimed sections must not also appear as generic forms.
        assert!(!text.contains("(section"));
        assert!(text.contains("(descriptions"));
        assert!(text.contains("(layout"));
        assert!(text.contains("(demo root"));
        let parsed: Registry = from_str(&text, Duration::from_secs(9), &codec()).unwrap();
        // The commits table preserves the head commit exactly.
        assert_eq!(parsed.head(&name("root")), Some(root_ca));
        assert_sections_survive(&parsed);
    }

    /// The same survives the inline-name format, where commits are
    /// synthesized on load.
    #[test]
    fn sections_round_trip_through_named_text() {
        let (reg, _root_ca) = test_registry();
        let text = to_string_named(&reg, &codec()).unwrap();
        assert!(!text.contains("(section"));
        let parsed: Registry = from_str(&text, Duration::from_secs(9), &codec()).unwrap();
        assert_sections_survive(&parsed);
    }

    /// A registry with no GUI sections emits no friendly forms.
    #[test]
    fn empty_sections_emit_no_forms() {
        let mut reg = Registry::default();
        let mut g = TestGraph::default();
        g.add_node(expr("(+ 1 1)"));
        commit_named(&mut reg, Duration::from_secs(1), &g, &name("only"));
        let text = to_string(&reg, &codec()).unwrap();
        assert!(!text.contains("(descriptions"));
        assert!(!text.contains("(layout"));
        assert!(!text.contains("(demo"));
    }
}
