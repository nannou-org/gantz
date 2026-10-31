/// The value-level codec for the app's node set. It maps typed nodes to and
/// from the registry's erased `NodeData` form and carries the set's `.gantz`
/// sugar.
///
/// These kinds, with the DSP node definitions of `gantz_plyphon::node_defs`,
/// are the app's wire-format manifest. Adding a node type to the app is one
/// line here. The `codec_covers_every_node_set_case` and `node_set_addr_pins`
/// gate tests enforce it.
pub fn codec() -> gantz_egui::node::NodeCodec {
    use gantz_egui::node::{def_kind, dyn_def_kind, kind};
    let kinds = [
        kind::<gantz_core::node::Apply>(),
        kind::<gantz_core::node::Branch>(),
        kind::<gantz_core::node::Delay>(),
        kind::<gantz_core::node::Expr>(),
        kind::<gantz_core::node::Identity>(),
        kind::<gantz_core::node::List>(),
        kind::<gantz_core::node::graph::Inlet>(),
        kind::<gantz_core::node::graph::Outlet>(),
        kind::<gantz_egui::node::Bang>(),
        kind::<gantz_io::Log>(),
        kind::<gantz_io::MainBang>(),
        kind::<gantz_egui::node::Number>(),
        kind::<gantz_egui::node::FnNamedRef>(),
        kind::<gantz_edit::NamedRef>(),
        kind::<gantz_egui::node::Bind>(),
        kind::<gantz_egui::node::Comment>(),
        kind::<bevy_gantz_egui::node::LoadBang>(),
        kind::<bevy_gantz_egui::node::UpdateBang>(),
        kind::<bevy_gantz_egui::node::TickBang>(),
        kind::<bevy_gantz_egui::node::Await>(),
        kind::<bevy_gantz_egui::node::Sleep>(),
        kind::<gantz_egui::node::Gui>(),
        kind::<gantz_egui::node::Inspect>(),
        kind::<gantz_egui::node::State>(),
        kind::<gantz_egui::node::Plot>(),
        def_kind::<gantz_pattern::Pmini>(),
        def_kind::<gantz_pattern::Pplot>(),
    ];
    let dsp = gantz_plyphon::node_defs().into_iter().map(dyn_def_kind);
    gantz_egui::node::NodeCodec::new(sugars(), kinds.into_iter().chain(dsp))
}

/// The `.gantz` keyword sugar of every domain's nodes.
pub fn sugars() -> gantz_format::Sugars {
    gantz_format::Sugars::default()
        .with(gantz_format::CoreSugar)
        .with(gantz_io::IoSugar)
        .with(gantz_egui::EguiSugar)
        .with(bevy_gantz_egui::BevySugar)
        .with(gantz_plyphon::PlyphonSugar)
        .with(gantz_pattern::PatternSugar)
}

/// The app's full builtin node set, composed from every domain's builtin
/// specs.
pub fn builtins() -> gantz_core::Builtins {
    gantz_core::Builtins::from_specs(
        gantz_core::node::builtins()
            .into_iter()
            .chain(gantz_io::builtins())
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
            gantz_edit::base::BaseSource {
                name: "rng",
                bytes: gantz_rng::BASE_BYTES,
                renames: gantz_rng::BASE_RENAMES,
            },
            gantz_edit::base::BaseSource {
                name: "pattern",
                bytes: gantz_pattern::BASE_BYTES,
                renames: gantz_pattern::BASE_RENAMES,
            },
        ]);
}
