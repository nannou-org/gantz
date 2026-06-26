use crate::{
    ContextMenuResponse, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, OpenLogs,
    Registry, SocketDoc, SocketKind,
};

impl NodeUi for gantz_std::log::Log {
    fn name(&self, _: &dyn Registry) -> &str {
        match self.level {
            log::Level::Error => "error",
            log::Level::Warn => "warn",
            log::Level::Info => "info",
            log::Level::Debug => "debug",
            log::Level::Trace => "trace",
        }
    }

    fn description(&self) -> Option<&'static str> {
        Some(match self.level {
            log::Level::Error => "Log a value at error level",
            log::Level::Warn => "Log a value at warning level",
            log::Level::Info => "Log a value at info level",
            log::Level::Debug => "Log a value at debug level",
            log::Level::Trace => "Log a value at trace level",
        })
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed = uictx.framed(|ui, _sockets| {
            let level = format!("{:?}", self.level).to_lowercase();
            ui.add(egui::Label::new(&level).selectable(false))
        });
        NodeUiResponse::new(framed)
    }

    fn inspector_rows(
        &mut self,
        _ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        let row_h = crate::widget::node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("logs");
            });
            row.col(|ui| {
                if ui
                    .button("open")
                    .on_hover_text("show the logs pane")
                    .clicked()
                {
                    resp.emit(OpenLogs);
                }
            });
        });
        resp
    }

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => {
                Some(SocketDoc::ty("any").with_description("value logged at this node's level"))
            }
            SocketKind::Output => None,
        }
    }

    fn context_menu(&mut self, _ctx: &mut NodeCtx, ui: &mut egui::Ui) -> ContextMenuResponse {
        let mut resp = ContextMenuResponse::default();
        if ui
            .button("open logs")
            .on_hover_text("show the logs pane")
            .clicked()
        {
            resp.emit(OpenLogs);
            ui.close();
        }
        resp
    }
}
