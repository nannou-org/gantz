/// The `.gantz` keyword sugar carrier composing
/// every domain's node sugar.
pub struct NodeSet;

impl gantz_format::NodeSugar for NodeSet {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![
            &gantz_format::CoreSugar,
            &gantz_std::StdSugar,
            &gantz_egui::EguiSugar,
            &bevy_gantz_egui::BevySugar,
            &gantz_plyphon::PlyphonSugar,
            &gantz_pattern::PatternSugar,
        ])
    }
}

/// The value-level codec for the app's node set. It maps typed nodes to and
/// from the registry's erased `NodeData` form and carries the set's `.gantz`
/// sugar.
///
/// This list is the app's wire-format manifest. Adding a node type to the app
/// is one line here. The `codec_covers_every_node_set_case` and
/// `node_set_addr_pins` gate tests enforce it.
pub fn codec() -> gantz_egui::node::NodeCodec {
    gantz_egui::ui_node_codec! {
        NodeSet {
            gantz_core::node::Apply,
            gantz_core::node::Branch,
            gantz_core::node::Delay,
            gantz_core::node::Expr,
            gantz_core::node::Identity,
            gantz_core::node::graph::Inlet,
            gantz_core::node::graph::Outlet,
            gantz_std::Bang,
            gantz_std::List,
            gantz_std::Log,
            gantz_std::Number,
            gantz_egui::node::FnNamedRef,
            gantz_egui::node::NamedRef,
            gantz_egui::node::Bind,
            gantz_egui::node::Comment,
            bevy_gantz_egui::node::UpdateBang,
            bevy_gantz_egui::node::TickBang,
            bevy_gantz_egui::node::Await,
            bevy_gantz_egui::node::Sleep,
            gantz_egui::node::Gui,
            gantz_egui::node::Inspect,
            gantz_egui::node::Plot,
            gantz_plyphon::UnitNode,
            gantz_plyphon::Out,
            gantz_plyphon::ScopeOut,
            gantz_plyphon::Pack,
            gantz_plyphon::Sum,
            gantz_plyphon::Unpack,
            gantz_plyphon::Bus,
            gantz_plyphon::Sample,
            gantz_plyphon::Buffer,
            gantz_plyphon::Envgen,
            gantz_pattern::Pmini,
            gantz_pattern::Pplot,
        }
    }
}

/// The app's full builtin node set, composed from every domain's builtin
/// specs.
pub fn builtins() -> gantz_core::Builtins {
    gantz_core::Builtins::from_specs(
        gantz_core::node::builtins()
            .into_iter()
            .chain(gantz_std::builtins())
            .chain(gantz_egui::builtins())
            .chain(bevy_gantz_egui::builtins())
            .chain(gantz_plyphon::builtins())
            .chain(gantz_pattern::builtins()),
    )
}

/// The Steel modules every head's engine registers, beyond the core set.
/// Base graphs `(require ...)` these, so any VM that compiles them, in the
/// app or in tests, registers the same list.
pub fn steel_modules() -> Vec<gantz_core::vm::SteelModule> {
    gantz_ui::modules()
        .iter()
        .chain(gantz_rng::modules())
        .chain(gantz_pattern::modules())
        .copied()
        .collect()
}

/// Contribute the domains that have no bevy plugin of their own.
///
/// The ui, rng and pattern domains are steel modules, and the rng and
/// pattern domains base sources, with no systems, so the app pushes their
/// contributions directly.
pub fn push_plain_domains(app: &mut bevy::app::App) {
    app.world_mut()
        .get_resource_or_init::<bevy_gantz::vm::SteelModules>()
        .0
        .extend(steel_modules());
    app.world_mut()
        .get_resource_or_init::<bevy_gantz_egui::base::BaseSources>()
        .0
        .extend([
            bevy_gantz_egui::base::BaseSource {
                name: "rng",
                bytes: gantz_rng::BASE_BYTES,
            },
            bevy_gantz_egui::base::BaseSource {
                name: "pattern",
                bytes: gantz_pattern::BASE_BYTES,
            },
        ]);
}
