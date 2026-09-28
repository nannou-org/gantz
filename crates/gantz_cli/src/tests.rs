use crate::Conf;
use gantz_egui::base::BaseSource;

mod cli;
#[cfg(feature = "collab")]
mod join;
#[cfg(feature = "collab")]
mod mirror;

/// The `.gantz` keyword sugar carrier of the test node set.
struct NodeSet;

impl gantz_format::NodeSugar for NodeSet {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![
            &gantz_format::CoreSugar,
            &gantz_io::IoSugar,
            &gantz_egui::EguiSugar,
        ])
    }
}

/// The core, std and egui node sets with the core base source. This is the
/// smallest node set that the core base graphs parse and compile with.
fn conf() -> Conf {
    Conf {
        codec: gantz_egui::ui_node_codec! {
            NodeSet {
                gantz_core::node::Apply,
                gantz_core::node::Branch,
                gantz_core::node::Delay,
                gantz_core::node::Expr,
                gantz_core::node::Identity,
                gantz_core::node::List,
                gantz_core::node::graph::Inlet,
                gantz_core::node::graph::Outlet,
                gantz_egui::node::Bang,
                gantz_io::Log,
                gantz_egui::node::Number,
                gantz_egui::node::FnNamedRef,
                gantz_egui::node::NamedRef,
                gantz_egui::node::Bind,
                gantz_egui::node::Comment,
                gantz_egui::node::Gui,
                gantz_egui::node::Inspect,
                gantz_egui::node::Plot,
            }
        },
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
        }],
        entrypoints: |get_node, graph| gantz_core::compile::push_pull_entrypoints(get_node, graph),
        org: "nannou-org",
        app: "gantz",
    }
}
