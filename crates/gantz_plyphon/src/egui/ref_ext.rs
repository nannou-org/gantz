//! The reference inspector's DSP extension. It is the `inline` toggle over
//! the [`DspRefExt`] data. The headless half lives in [`crate::ref_ext`].

use crate::ref_ext::{DSP_REF_EXT_KEY, DspRefExt};
use gantz_ca::ContentAddr;
use gantz_egui::node::{NamedRef, RefExtUi};
use gantz_egui::widget::node_inspector;
use gantz_egui::{InspectorRowsResponse, NodeCtx};
use std::collections::HashSet;
use std::sync::Arc;

/// The DSP domain's [`NamedRef`] inspector extension. It is an `inline`
/// toggle for references whose graph contains DSP nodes, directly or
/// transitively.
#[derive(Debug, Default)]
pub struct DspRefExtUi {
    /// The graph addresses of DSP graphs, precomputed from the stored
    /// registry data. See [`dsp_graphs`](crate::dsp_graphs).
    pub dsp_graphs: Arc<HashSet<ContentAddr>>,
}

impl RefExtUi for DspRefExtUi {
    fn inspector_rows(
        &self,
        named: &mut NamedRef,
        _ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        if !self.dsp_graphs.contains(&named.content_addr()) {
            return resp;
        }
        let row_h = node_inspector::table_row_h(body.ui_mut());
        let mut inline = named
            .ext_as::<DspRefExt>(DSP_REF_EXT_KEY)
            .unwrap_or_default()
            .inline;
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("inline");
            });
            row.col(|ui| {
                if ui
                    .checkbox(&mut inline, "")
                    .on_hover_text(
                        "Inline this reference's DSP nodes into the parent synthdef \
                         instead of spawning them from the shared per-variant \
                         definition (the default). Inlined copies compile their own \
                         units; instanced copies share one definition.",
                    )
                    .changed()
                {
                    // Stored only when non-default so a default-configured
                    // reference keeps its address.
                    if inline {
                        named
                            .set_ext(DSP_REF_EXT_KEY, &DspRefExt { inline })
                            .expect("`DspRefExt` is datum-representable");
                    } else {
                        named.remove_ext(DSP_REF_EXT_KEY);
                    }
                    resp.mark_changed();
                }
            });
        });
        resp
    }
}
