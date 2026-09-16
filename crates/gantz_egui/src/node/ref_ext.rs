//! The domain seam for extending the [`NamedRef`] inspector.
//!
//! A domain stores its per-reference data in the underlying
//! [`gantz_core::node::Ref`]'s ext slot. See
//! [`gantz_core::node::Ref::set_ext`]. It exposes the data by providing a
//! [`RefExtUi`] to the [`crate::widget::Gantz`] widget via
//! [`crate::widget::Gantz::ref_ext_uis`]. The [`NamedRef`] inspector appends
//! each provider's rows after its own.

use crate::node::NamedRef;
use crate::{InspectorRowsResponse, NodeCtx};

/// An inspector extension for [`NamedRef`] nodes, provided by a domain.
///
/// Implementations self-gate. Return [`InspectorRowsResponse::default`] for
/// references the domain has no interest in. For example, a DSP extension
/// checks whether the referenced graph contains DSP nodes.
pub trait RefExtUi {
    /// Append the domain's rows to a `NamedRef`'s inspector table.
    ///
    /// Edits typically read and write the reference's ext data through
    /// [`NamedRef::ext_as`], [`NamedRef::set_ext`] and
    /// [`NamedRef::remove_ext`]. They mark the response changed so the
    /// ordinary commit pipeline persists them.
    fn inspector_rows(
        &self,
        named: &mut NamedRef,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeUi;

    struct StubExt;

    impl RefExtUi for StubExt {
        fn inspector_rows(
            &self,
            _named: &mut NamedRef,
            _ctx: &mut NodeCtx,
            body: &mut egui_extras::TableBody,
        ) -> InspectorRowsResponse {
            let mut resp = InspectorRowsResponse::default();
            body.row(18.0, |mut row| {
                row.col(|ui| {
                    ui.label("stub");
                });
                row.col(|_| {});
            });
            resp.mark_changed();
            resp.emit("stub-payload".to_string());
            resp
        }
    }

    /// The `NamedRef` inspector merges each provided extension's changed flag
    /// and payloads into its own response.
    #[test]
    fn named_ref_inspector_merges_ext_ui_responses() {
        let registry = gantz_ca::Registry::default();
        let graphs = gantz_core::data::ReifiedGraphs::new();
        let builtins = gantz_core::Builtins::default();
        let instances = crate::node::UiBuiltins::default();
        let codec = crate::test_node::codec();
        let env = crate::Env {
            registry: &registry,
            builtins: &builtins,
            codec: &codec,
            graphs: &graphs,
            instances: &instances,
        };
        let mut vm = gantz_core::steel::steel_vm::engine::Engine::new_base();
        let mut named = NamedRef::new(
            "x".parse().unwrap(),
            gantz_core::node::Ref::new([0u8; 32].into()),
        );
        let exts: [&dyn RefExtUi; 1] = [&StubExt];
        let mut writes = Vec::new();

        let mut merged = InspectorRowsResponse::default();
        egui::__run_test_ui(|ui| {
            egui_extras::TableBuilder::new(ui)
                .column(egui_extras::Column::auto())
                .column(egui_extras::Column::remainder())
                .body(|mut body| {
                    let mut ctx = crate::NodeCtx::new(
                        &env,
                        &[0][..],
                        &[],
                        &[],
                        &exts[..],
                        &mut vm,
                        &mut writes,
                    );
                    merged = named.inspector_rows(&mut ctx, &mut body);
                });
        });
        assert!(merged.changed, "ext change must merge into the response");
        let payload = merged
            .payloads
            .into_iter()
            .find_map(|p| p.downcast::<String>().ok())
            .expect("ext payload must merge into the response");
        assert_eq!(payload, "stub-payload");
    }
}
