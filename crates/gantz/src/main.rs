use bevy::{
    prelude::*,
    window::{PrimaryWindow, Window},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_gantz::{
    FocusedHead, GantzPlugin, HeadRef, HeadTabOrder, OpenHead, Registry, WorkingGraph,
    debounced_input::{DebouncedInputEvent, DebouncedInputPlugin},
    timestamp,
};
use bevy_gantz_egui::{BuiltinNodes, GantzEguiPlugin, TraceCapture};
use bevy_pkv::PkvStore;
use storage::Pkv;

#[cfg(test)]
mod headless;
mod node;
mod persist;
mod storage;
mod window;

fn main() {
    // cpal's AudioWorklet backend on the web re-instantiates this wasm module
    // on the audio thread and re-runs `main` there. Only boot the app on the
    // main browser thread.
    if bevy_gantz_plyphon::on_worklet_thread() {
        return;
    }
    let mut app = App::new();
    // Domains with no bevy plugin, such as the pattern domain.
    node::push_plain_domains(&mut app);
    app.add_plugins(GantzPlugin)
        .add_plugins(GantzEguiPlugin::default())
        .add_plugins(bevy_gantz_plyphon::PlyphonPlugin::default())
        // A builtin that fails to reify is a node-set composition error, so
        // fail loudly at startup.
        .insert_resource({
            let (builtins, errs) = BuiltinNodes::reify(node::builtins(), &node::codec());
            assert!(errs.is_empty(), "builtins failed to reify: {errs:?}");
            builtins
        })
        // The app's node codec is the node-set manifest for the reify and
        // erase seam and the `.gantz` parse and export paths.
        .insert_resource(bevy_gantz_egui::NodeCodecRes(node::codec()))
        .add_plugins(DefaultPlugins.set(log_plugin()).set(window::plugin()))
        .add_plugins(EguiPlugin::default())
        // Drives both layout settling and the registry persist.
        .add_plugins(DebouncedInputPlugin::<DebouncedInputEvent>::new(0.25))
        .add_plugins(persist::PersistPlugin)
        .insert_resource(Pkv::new(PkvStore::new("nannou-org", &store_name())))
        .add_systems(
            Startup,
            (
                setup_camera,
                setup_window,
                setup_resources,
                bevy_gantz_egui::base::load
                    .after(setup_resources)
                    .before(setup_open),
                setup_open.after(setup_resources),
                bevy_gantz_egui::prune_unused
                    .after(setup_resources)
                    .after(setup_open),
            ),
        )
        .add_systems(EguiPrimaryContextPass, load_egui_memory);

    // Added after `GantzEguiPlugin`, because the collab plugin's
    // payload-dispatcher overrides rely on last-registration-wins.
    #[cfg(feature = "collab")]
    app.add_plugins(bevy_gantz_collab::CollabPlugin)
        .add_systems(Startup, setup_collab_identity);

    // Native OS windows for popped-out panes. On web the widget keeps drawing
    // popped-out panes as in-canvas `egui::Window`s.
    #[cfg(not(target_arch = "wasm32"))]
    app.add_plugins(bevy_gantz_egui::pane_window::PaneWindowPlugin);

    app.run();
}

/// The persistent store name. `GANTZ_STORE` overrides it, so a second
/// instance such as a local collab peer gets its own state instead of
/// clashing over the store's lock.
fn store_name() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(name) = std::env::var("GANTZ_STORE") {
        if !name.is_empty() {
            return name;
        }
    }
    "gantz".to_string()
}

fn log_plugin() -> bevy::log::LogPlugin {
    bevy::log::LogPlugin {
        custom_layer: move |app| {
            // `get_resource_or_init` shares the instance with
            // `GantzEguiPlugin`'s `init_resource`, so plugin order does not
            // matter.
            let capture = app.world_mut().get_resource_or_init::<TraceCapture>();
            Some(Box::new(capture.0.clone().layer()))
        },
        ..Default::default()
    }
}

fn setup_camera(mut cmds: Commands) {
    cmds.spawn(Camera2d);
}

/// Restore the persisted window size. No-op on web.
fn setup_window(storage: Res<Pkv>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    if let Ok(mut window) = windows.single_mut() {
        window::apply_saved_size(&*storage, &mut window);
    }
}

fn setup_resources(storage: Res<Pkv>, mut cmds: Commands) {
    let registry: Registry = bevy_gantz::storage::load_registry(&*storage);
    // Seed the persist tracker before `base::load` merges base graphs, so they
    // are written on first persist, and before `prune_unused`, so prunes are
    // detected on the first incremental save.
    let persisted = bevy_gantz::storage::PersistedRegistry::from_registry(&registry);
    let gui_state = bevy_gantz_egui::storage::load_gui_state(&*storage);
    // Reify the loaded registry's graphs so typed reads are served from the
    // first frame.
    let mut cache = bevy_gantz_egui::GraphCache::default();
    bevy_gantz_egui::refresh_cache(&registry, &mut cache, &node::codec());
    cmds.insert_resource(registry);
    cmds.insert_resource(cache);
    cmds.insert_resource(persisted);
    cmds.insert_resource(gui_state);
}

/// Load the user's collaborative identity, or generate and persist one.
#[cfg(feature = "collab")]
fn setup_collab_identity(mut storage: ResMut<Pkv>, mut cmds: Commands) {
    let identity = match bevy_gantz_collab::storage::load_identity(&*storage) {
        Some(identity) => identity,
        None => {
            let identity = gantz_collab::Identity::generate();
            bevy_gantz_collab::storage::save_identity(&mut *storage, &identity);
            identity
        }
    };
    cmds.insert_resource(bevy_gantz_collab::CollabIdentity(identity));
}

fn setup_open(
    storage: Res<Pkv>,
    mut registry: ResMut<Registry>,
    mut cmds: Commands,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
) {
    let loaded = bevy_gantz_egui::storage::load_open(&*storage, &mut *registry, timestamp());
    let focused_head = bevy_gantz::storage::load_focused_head(&*storage);

    // `OpenHead`'s required components cover the compile outcome. `GraphView`'s
    // cover the rest of the per-head GUI state. `vm::sync` initializes the VMs
    // on the first `Update`.
    for (head, graph, head_view) in loaded {
        let is_focused = focused_head.as_ref() == Some(&head);
        let entity = cmds
            .spawn((OpenHead, HeadRef(head), WorkingGraph(graph), head_view))
            .id();

        tab_order.push(entity);

        if is_focused || (**focused).is_none() {
            **focused = Some(entity);
        }
    }
}

/// Load egui memory from storage once on first frame.
fn load_egui_memory(mut ctxs: EguiContexts, mut storage: ResMut<Pkv>, mut loaded: Local<bool>) {
    if !*loaded {
        if let Ok(ctx) = ctxs.ctx_mut() {
            bevy_gantz_egui::storage::load_egui_memory(&mut *storage, ctx);
            *loaded = true;
        }
    }
}

#[cfg(test)]
mod tests {
    const BASE_GANTZ: &[u8] = gantz_base::BYTES;

    #[test]
    fn base_gantz_deserializes() {
        let _registry: gantz_ca::Registry =
            gantz_egui::export::parse_export(BASE_GANTZ, &super::node::codec())
                .expect("valid .gantz");
    }
}
