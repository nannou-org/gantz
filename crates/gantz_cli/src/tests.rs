use crate::Conf;
use gantz_edit::base::BaseSource;

mod cli;
#[cfg(feature = "collab")]
mod join;
mod logs;
#[cfg(feature = "collab")]
mod mirror;
#[cfg(feature = "collab")]
mod vault;

/// The core, std and egui node sets with the core base source. This is the
/// smallest node set that the core base graphs parse and compile with.
fn conf() -> Conf {
    Conf {
        codec: gantz_egui::node::NodeCodec::new(
            gantz_format::Sugars::default()
                .with(gantz_format::CoreSugar)
                .with(gantz_io::IoSugar)
                .with(gantz_egui::EguiSugar),
            [
                gantz_egui::node::kind::<gantz_core::node::Apply>(),
                gantz_egui::node::kind::<gantz_core::node::Branch>(),
                gantz_egui::node::kind::<gantz_core::node::Delay>(),
                gantz_egui::node::kind::<gantz_core::node::Expr>(),
                gantz_egui::node::kind::<gantz_core::node::Identity>(),
                gantz_egui::node::kind::<gantz_core::node::List>(),
                gantz_egui::node::kind::<gantz_core::node::graph::Inlet>(),
                gantz_egui::node::kind::<gantz_core::node::graph::Outlet>(),
                gantz_egui::node::kind::<gantz_egui::node::Bang>(),
                gantz_egui::node::kind::<gantz_io::Log>(),
                gantz_egui::node::kind::<gantz_io::MainBang>(),
                gantz_egui::node::kind::<gantz_egui::node::Number>(),
                gantz_egui::node::kind::<gantz_egui::node::FnNamedRef>(),
                gantz_egui::node::kind::<gantz_edit::NamedRef>(),
                gantz_egui::node::kind::<gantz_egui::node::Bind>(),
                gantz_egui::node::kind::<gantz_egui::node::Comment>(),
                gantz_egui::node::kind::<gantz_egui::node::Gui>(),
                gantz_egui::node::kind::<gantz_egui::node::Inspect>(),
                gantz_egui::node::kind::<gantz_egui::node::State>(),
                gantz_egui::node::kind::<gantz_egui::node::Plot>(),
            ],
        ),
        builtins: gantz_core::Builtins::from_specs(
            gantz_core::node::builtins()
                .into_iter()
                .chain(gantz_io::builtins())
                .chain(gantz_egui::builtins()),
        ),
        steel_modules: gantz_ui::modules().to_vec(),
        base_sources: vec![BaseSource {
            name: "gantz",
            bytes: gantz_base::BYTES,
            renames: gantz_base::RENAMES,
        }],
        entrypoints: |get_node, graph| gantz_core::compile::push_pull_entrypoints(get_node, graph),
        org: "nannou-org",
        app: "gantz",
        build: concat!("gantz ", env!("CARGO_PKG_VERSION")),
        live: None,
    }
}
