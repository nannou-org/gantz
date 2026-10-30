//! `gantz run --live`: a headless app that keeps a graph running.
//!
//! The app runs the gantz plugins and the domains of the app with no window.
//! It opens the graph as a head and fires its main! nodes, or a push source,
//! once the graph compiles. Then `load!`, `tick!`, `update!`, `await` and the
//! DSP nodes run as in the app, and each value of a `log` node prints as one
//! line.

use crate::node;
use bevy::app::AppExit;
use bevy::prelude::*;
use bevy_gantz::head::{HeadRef, HeadVms, Module, OpenEvent, OpenHead};
use bevy_gantz::vm::{CompileConfig, CompiledInputs, EvalEntryEvent, Inputs, VmSetups};
use bevy_gantz::{EntrypointSet, GantzPlugin, Registry};
use bevy_gantz_egui::{BuiltinNodes, GraphCache, NodeCodecRes, RuntimePlugin};
use std::time::{Duration, Instant};

/// The time between updates.
const FRAME: Duration = Duration::from_millis(16);

/// The evaluation to fire once the graph compiles.
#[derive(Resource)]
struct Fire {
    entry: Option<gantz_core::compile::Entrypoint>,
    /// The paths of the main! nodes, whose state takes `args`.
    main: Vec<Vec<gantz_core::node::Id>>,
    args: Vec<String>,
    fired: bool,
}

/// Keep the graph of `live` running, and give the exit code.
pub fn run(live: gantz_cli::Live) -> i32 {
    let duration = live.duration;
    let mut app = app(live, |line| println!("{line}"));
    let start = Instant::now();
    loop {
        let frame = Instant::now();
        app.update();
        if let Some(exit) = app.should_exit() {
            return match exit {
                AppExit::Success => 0,
                AppExit::Error(code) => i32::from(code.get()),
            };
        }
        if duration.is_some_and(|d| start.elapsed() >= d) {
            return 0;
        }
        std::thread::sleep(FRAME.saturating_sub(frame.elapsed()));
    }
}

/// A headless app that runs the graph of `live`, and gives each value of a
/// `log` node to `print`.
pub fn app(live: gantz_cli::Live, print: impl Fn(String) + Clone + Send + Sync + 'static) -> App {
    let gantz_cli::Live {
        registry,
        name,
        entry,
        main,
        args,
        duration: _,
    } = live;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    node::push_plain_domains(&mut app);
    let codec = node::codec();
    let (builtins, errs) = BuiltinNodes::reify(node::builtins(), &codec);
    assert!(errs.is_empty(), "builtins failed to reify: {errs:?}");
    app.add_plugins((
        GantzPlugin,
        RuntimePlugin,
        bevy_gantz_plyphon::PlyphonPlugin::default(),
    ))
    .insert_resource(builtins)
    .insert_resource(NodeCodecRes(codec))
    .insert_resource(Registry(registry))
    .insert_resource(Fire {
        entry,
        main,
        args,
        fired: false,
    })
    .add_systems(
        Update,
        fire.in_set(EntrypointSet)
            .after(bevy_gantz_egui::node::load_bang::drive_load_bangs),
    );
    app.world_mut()
        .get_resource_or_init::<VmSetups>()
        .0
        .push(Box::new(move |vm| {
            let print = print.clone();
            gantz_io::log::register_sink(vm, move |_level, _path, val| print(val.to_string()));
        }));
    app.world_mut()
        .resource_scope::<GraphCache, _>(|world, mut cache| {
            let registry = world.resource::<Registry>();
            bevy_gantz_egui::refresh_cache(registry, &mut cache, &codec);
        });
    app.world_mut()
        .trigger(OpenEvent(gantz_ca::Head::Branch(name)));
    app.finish();
    app.cleanup();
    app
}

/// Fires the evaluation of [`Fire`] once the graph compiles. A graph that
/// fails to compile ends the run.
fn fire(
    mut fire: ResMut<Fire>,
    config: Res<CompileConfig>,
    registry: Res<Registry>,
    heads: Query<(Entity, &HeadRef, &CompiledInputs, &Module), With<OpenHead>>,
    mut vms: NonSendMut<HeadVms>,
    mut cmds: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if fire.fired {
        return;
    }
    let Ok((head, head_ref, compiled, module)) = heads.single() else {
        return;
    };
    let Some(graph) = registry.head_commit(&head_ref.0).map(|c| c.graph) else {
        return;
    };
    if compiled.0
        != Some(Inputs {
            graph,
            config: config.0,
        })
    {
        return;
    }
    fire.fired = true;
    if let Some(error) = &module.error {
        eprintln!("{error}");
        exit.write(AppExit::from_code(1));
        return;
    }
    let Some(entry) = fire.entry.clone() else {
        return;
    };
    let Some(vm) = vms.get_mut(&head) else {
        return;
    };
    for path in &fire.main {
        if let Err(e) = gantz_core::node::state::update(vm, path, fire.args.clone()) {
            eprintln!("main! node {path:?}: {e}");
        }
    }
    cmds.trigger(EvalEntryEvent {
        head,
        entrypoint: entry,
        time: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::sync::mpsc;

    fn name() -> gantz_ca::Name {
        "g".parse().unwrap()
    }

    /// A live app of the graph `g` in `registry`, which updates by `step`,
    /// and the receiver of the lines it prints.
    fn live(
        registry: gantz_ca::Registry,
        entry: Option<gantz_core::compile::Entrypoint>,
        main: Vec<Vec<gantz_core::node::Id>>,
        args: Vec<String>,
        step: Duration,
    ) -> (App, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel();
        let live = gantz_cli::Live {
            registry,
            name: name(),
            entry,
            main,
            args,
            duration: None,
        };
        let mut app = app(live, move |line| {
            let _ = tx.send(line);
        });
        app.insert_resource(TimeUpdateStrategy::ManualDuration(step));
        (app, rx)
    }

    fn parse(text: &str) -> gantz_ca::Registry {
        gantz_edit::format::from_str(text, Duration::ZERO, node::codec().data()).expect("parse")
    }

    fn update(app: &mut App, n: usize) {
        for _ in 0..n {
            app.update();
        }
    }

    /// A `tick!` drives the graph with no main! node.
    #[test]
    fn a_tick_drives_a_live_graph() {
        let registry = parse("(graph g (t (tick-bang #:rate 20)) (l log) (-> t l))");
        let (mut app, rx) = live(registry, None, vec![], vec![], Duration::from_millis(50));
        update(&mut app, 6);
        let lines: Vec<String> = rx.try_iter().collect();
        assert!(lines.len() >= 3, "ticks at 20 Hz print: {lines:?}");
        assert!(lines.iter().all(|l| l == "0.05"), "{lines:?}");
    }

    /// The main! nodes fire once with the program arguments.
    #[test]
    fn main_fires_once_with_the_args() {
        let registry = parse("(graph g (m main!) (l log) (-> m l))");
        let entry = gantz_io::main_bang::entrypoint_of([0]);
        let args = vec!["a".to_string(), "b c".to_string()];
        let step = Duration::from_millis(16);
        let (mut app, rx) = live(registry, entry, vec![vec![0]], args, step);
        update(&mut app, 4);
        let lines: Vec<String> = rx.try_iter().collect();
        assert_eq!(lines, ["(\"a\" \"b c\")"]);
    }
}
