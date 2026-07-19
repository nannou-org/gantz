//! The GUI Debug pane's editor: a Steel tree literal that evaluates and
//! decodes into a [`gantz_ui::Element`] tree as it is typed.
//!
//! The pane renders the decoded tree through the [`ui_tree`][crate::ui_tree]
//! interpreter against the focused head's live VM, making it both a
//! playground for the GUI vocabulary and a live harness: bindings resolve
//! into the focused graph's node state, and pushes fire its entrypoints.

use crate::{Env, node};
use petgraph::visit::{IntoNodeReferences, NodeRef};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

/// The tree seeded into a fresh editor: self-contained layout/display
/// elements plus a couple of bindings into the focused graph (nodes 0 and 1)
/// to demonstrate the live harness. Evaluated by `Engine::new_base` (no
/// prelude), hence the quote.
const SEED: &str = r#"'(col
  (frame (@ (title "gui debug"))
    (label "edit this tree - it renders live")
    (sep)
    (row (dialer (@ (bind (0)) (label "node 0")))
         (button (@ (bind (1)))))
    (value (@ (bind (0))))))
"#;

/// The evaluated + decoded state of the editor text, cached by text hash so
/// the Steel engine only runs when the text changes.
pub struct Cache {
    text_hash: u64,
    /// The engine's error when the text failed to evaluate to a value.
    pub eval_err: Option<String>,
    /// The decoded tree + warnings when evaluation yielded a value.
    pub decoded: Option<gantz_ui::Decoded>,
}

/// The editor's response and its current evaluation state.
pub struct TreeEditOutput {
    /// The cached evaluation of the editor's current text.
    pub cache: Arc<Cache>,
    /// The text editor's response.
    pub response: egui::Response,
}

/// A multiline Steel editor whose text is evaluated (in a scratch base
/// engine) and decoded into an element tree whenever it changes. The WIP
/// text and the evaluation cache persist in egui temp memory under `id`.
pub fn tree_editor(id: egui::Id, ui: &mut egui::Ui) -> TreeEditOutput {
    let code_id = id.with("code");
    let cache_id = id.with("cache");

    let mut code: String = ui
        .memory_mut(|m| m.data.get_temp(code_id))
        .unwrap_or_else(|| SEED.to_string());

    let language = "scm";
    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut layout_job = egui_extras::syntax_highlighting::highlight(
            ui.ctx(),
            ui.style(),
            &theme,
            buf.as_str(),
            language,
        );
        layout_job.wrap.max_width = wrap_width;
        ui.fonts_mut(|fonts| fonts.layout_job(layout_job))
    };

    let font_id = egui::FontSelection::from(egui::TextStyle::Monospace).resolve(ui.style());
    let response = ui
        .add(
            egui::TextEdit::multiline(&mut code)
                .id(id)
                .code_editor()
                .font(font_id)
                .desired_width(ui.available_width())
                .layouter(&mut layouter),
        )
        .on_hover_text(
            "A quoted tree literal, evaluated by a scratch Steel engine on \
             the UI thread whenever the text changes (no prelude - primitive \
             forms only)",
        );

    // Re-evaluate only when the text changes.
    let text_hash = {
        let mut h = DefaultHasher::new();
        code.hash(&mut h);
        h.finish()
    };
    let cache: Option<Arc<Cache>> = ui.memory_mut(|m| m.data.get_temp(cache_id));
    let cache = match cache {
        Some(cache) if cache.text_hash == text_hash => cache,
        _ => Arc::new(evaluate(text_hash, &code)),
    };

    ui.memory_mut(|m| {
        m.data.insert_temp(code_id, code);
        m.data.insert_temp(cache_id, cache.clone());
    });

    TreeEditOutput { cache, response }
}

/// Evaluate the editor text in a scratch base engine and decode the last
/// yielded value into an element tree.
fn evaluate(text_hash: u64, code: &str) -> Cache {
    let mut engine = steel::steel_vm::engine::Engine::new_base();
    let (eval_err, decoded) = match engine.run(code.to_string()) {
        Err(e) => (Some(e.to_string()), None),
        Ok(vals) => match vals.last() {
            None => (Some("the text evaluates to no value".to_string()), None),
            Some(val) => (
                None,
                Some(gantz_ui::codec::steel::decode(
                    val,
                    &gantz_ui::Limits::default(),
                )),
            ),
        },
    };
    Cache {
        text_hash,
        eval_err,
        decoded,
    }
}

/// The output count of every node in `g`, backing the interpreter's
/// push-eval resolver (a push entry fn's identity covers the count).
///
/// Nodes reify transiently through the codec; a weight that fails to reify
/// (an unknown tag) reports no count.
pub fn node_output_counts(
    registry: &Env<'_>,
    codec: &node::NodeCodec,
    g: &gantz_ca::DataGraph,
) -> HashMap<node::Id, usize> {
    use gantz_core::Node;
    let get_node = |ca: &gantz_ca::ContentAddr| registry.node(ca);
    let ctx = gantz_core::node::MetaCtx::new(&get_node);
    g.node_references()
        .filter_map(|n| {
            let inst = codec.reify_ui(n.weight()).ok()?;
            Some((n.id().index(), inst.node.n_outputs(ctx)))
        })
        .collect()
}
