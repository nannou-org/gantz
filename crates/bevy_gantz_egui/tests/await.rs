//! End-to-end tests for the `await` node: the state/branch protocol in a bare
//! VM, and result delivery through the bevy driver in a headless app.

use bevy_gantz::task::{GantzTask, TaskHandle};
use bevy_gantz_egui::node::{Await, Sleep, await_};
use gantz_core::{
    Edge, Node,
    compile::{Entrypoint, EvalKind, entry_fn_name, push_pull_entrypoints},
    node::{self, WithPushEval},
};
use std::fmt::Debug;
use steel::SteelVal;
use steel::rvals::IntoSteelVal;
use steel::steel_vm::engine::Engine;

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A no-op node lookup function for tests that don't need it.
fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

/// The push entrypoint whose single source is the node at `path`.
fn push_entry<'a>(eps: &'a [Entrypoint], path: &[usize]) -> &'a Entrypoint {
    eps.iter()
        .find(|ep| {
            ep.0.iter()
                .any(|s| s.kind == EvalKind::Push && s.path == path)
        })
        .expect("push entrypoint")
}

/// Call the given entrypoint's generated entry fn.
fn call(vm: &mut Engine, ep: &Entrypoint) {
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .expect("entry fn errored");
}

/// The state of the node at the given root index.
fn state(vm: &Engine, ix: usize) -> SteelVal {
    node::state::extract_value(vm, &[ix])
        .expect("state read")
        .expect("state present")
}

/// The handle stashed in an await node's pending state pair, sharing the cell
/// of the one in state (the driver polls through such a handle in place).
fn stashed_handle(vm: &Engine, ix: usize) -> TaskHandle {
    await_::pending_handle(&state(vm, ix)).expect("await state should be a pending pair")
}

/// A `(list 2 <handle>)` pending pair over a fresh handle for the given task,
/// as the await expr stashes. Returns the pair and a clone of the handle.
fn pending_pair(task: GantzTask) -> (SteelVal, TaskHandle) {
    let handle = TaskHandle::new(task);
    let pair = SteelVal::ListV(
        [
            SteelVal::IntV(2),
            handle.clone().into_steelval().expect("handle to steelval"),
        ]
        .into_iter()
        .collect(),
    );
    (pair, handle)
}

/// The whole state/branch protocol in a bare VM, under both compile configs:
/// a received task is stashed and swallows the push; a driver-written value
/// pair fires only the value output; an error pair only the error output; and
/// a non-task input passes straight through.
#[test]
fn await_delivers_value_error_and_passthrough() {
    for config in [
        gantz_core::compile::Config::default(),
        gantz_core::compile::Config {
            validate_ir: true,
            emit_all_node_fns: true,
        },
    ] {
        let mut g = petgraph::graph::DiGraph::new();
        let mut sleep_node = Sleep::default();
        sleep_node.set_duration(0.0);
        let push =
            g.add_node(Box::new(node::expr("7").unwrap().with_push_eval()) as Box<dyn DebugNode>);
        let sleep = g.add_node(Box::new(sleep_node) as Box<_>);
        let await_n = g.add_node(Box::new(Await) as Box<_>);
        let val_sink = g.add_node(Box::new(gantz_egui::node::Inspect) as Box<_>);
        let err_sink = g.add_node(Box::new(gantz_egui::node::Inspect) as Box<_>);
        let push2 =
            g.add_node(Box::new(node::expr("9").unwrap().with_push_eval()) as Box<dyn DebugNode>);
        g.add_edge(push, sleep, Edge::from((0, 0)));
        g.add_edge(sleep, await_n, Edge::from((0, 0)));
        g.add_edge(push2, await_n, Edge::from((0, 0)));
        g.add_edge(await_n, val_sink, Edge::from((0, 0)));
        g.add_edge(await_n, err_sink, Edge::from((1, 0)));

        let eps = push_pull_entrypoints(&no_lookup, &g);
        let (mut vm, _compiled) = gantz_core::vm::init(&no_lookup, &g, &eps, &config)
            .unwrap_or_else(|e| panic!("init: {}", gantz_core::vm::error_chain(&e)));

        // Push a value through `sleep`: the task reaches `await`, which
        // stashes it and swallows the evaluation - neither sink fires.
        call(&mut vm, push_entry(&eps, &[push.index()]));
        assert_eq!(state(&vm, val_sink.index()), SteelVal::Void);
        assert_eq!(state(&vm, err_sink.index()), SteelVal::Void);

        // The stashed zero-duration task resolves to the pushed value on the
        // first in-place check, leaving the handle in state.
        let result = stashed_handle(&vm, await_n.index())
            .check()
            .expect("ready")
            .expect("resolves ok");
        assert_eq!(result, SteelVal::IntV(7));

        // Driver delivery: write the value pair, fire the await entry fn -
        // only the value output fires.
        node::state::update_value(&mut vm, &[await_n.index()], await_::value_pair(result))
            .expect("state write");
        call(&mut vm, push_entry(&eps, &[await_n.index()]));
        assert_eq!(state(&vm, val_sink.index()), SteelVal::IntV(7));
        assert_eq!(state(&vm, err_sink.index()), SteelVal::Void);

        // An error pair fires only the error output.
        node::state::update_value(
            &mut vm,
            &[await_n.index()],
            await_::error_pair("boom".to_string()),
        )
        .expect("state write");
        call(&mut vm, push_entry(&eps, &[await_n.index()]));
        assert_eq!(state(&vm, val_sink.index()), SteelVal::IntV(7));
        assert_eq!(
            state(&vm, err_sink.index()),
            SteelVal::StringV("boom".into())
        );

        // A non-task input passes straight through the value output.
        call(&mut vm, push_entry(&eps, &[push2.index()]));
        assert_eq!(state(&vm, val_sink.index()), SteelVal::IntV(9));
    }
}

/// The test app's `.gantz` sugar carrier (required by the codec macro; this
/// test never parses text).
struct NodeSet;

impl gantz_format::NodeSugar for NodeSet {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![&gantz_format::CoreSugar])
    }
}

/// The codec over the test's node set, through which the runtime's
/// reified-graph cache serves the stored graph as typed nodes.
fn codec() -> gantz_egui::node::NodeCodec {
    gantz_egui::ui_node_codec! {
        NodeSet {
            bevy_gantz_egui::node::Await,
            bevy_gantz_egui::node::Sleep,
            gantz_egui::node::Inspect,
        }
    }
}

/// A headless app with the gantz plugin, the test codec, and the `vm::sync` +
/// `drive_awaits` systems - the minimal plumbing the await driver needs.
fn task_test_app() -> bevy_app::App {
    use bevy_app::{App, TaskPoolPlugin, Update};
    use bevy_ecs::prelude::IntoScheduleConfigs;
    use bevy_gantz::{EntrypointSet, GantzPlugin, VmSet};

    let mut app = App::new();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(GantzPlugin)
        .insert_resource(bevy_gantz_egui::NodeCodecRes(codec()))
        .init_resource::<bevy_gantz_egui::GraphCache>()
        .init_resource::<bevy_gantz_egui::BuiltinNodes>()
        .add_systems(
            Update,
            (
                bevy_gantz_egui::vm::sync.in_set(VmSet),
                await_::drive_awaits.after(VmSet).in_set(EntrypointSet),
            ),
        );
    app.world_mut()
        .get_resource_or_init::<bevy_gantz_egui::vm::EntrypointFns>()
        .0
        .push(Box::new(|get_node, graph| {
            gantz_core::compile::push_pull_entrypoints(get_node, graph)
        }));
    app
}

/// Reify the registry's committed graphs into the app's graph cache.
fn refresh_app_cache(app: &mut bevy_app::App) {
    app.world_mut()
        .resource_scope::<bevy_gantz_egui::GraphCache, _>(|world, mut cache| {
            let registry = world.resource::<bevy_gantz::Registry>();
            bevy_gantz_egui::refresh_cache(registry, &mut cache, &codec());
        });
}

/// The full bevy plumbing, headless: `sleep -> await -> inspect` built in
/// code, compiled by `vm::sync`, the sleep entrypoint fired, and the result
/// delivered by `drive_awaits` once the duration elapses.
#[test]
fn driver_delivers_sleep_result_through_app() {
    use bevy_ecs::prelude::*;
    use bevy_gantz::{Registry, head, timestamp};
    use std::time::{Duration, Instant};

    let mut app = task_test_app();

    // Build `sleep(0.05) -> await -> inspect` as stored (erased) data.
    let mut sleep_node = Sleep::default();
    sleep_node.set_duration(0.05);
    let mut dg = gantz_ca::DataGraph::default();
    let sleep = dg.add_node(gantz_core::data::erase_node_typed(&sleep_node).unwrap());
    let await_n = dg.add_node(gantz_core::data::erase_node_typed(&Await).unwrap());
    let inspect =
        dg.add_node(gantz_core::data::erase_node_typed(&gantz_egui::node::Inspect).unwrap());
    dg.add_edge(sleep, await_n, gantz_ca::Edge::from((0, 0)));
    dg.add_edge(await_n, inspect, gantz_ca::Edge::from((0, 0)));

    // Commit it, reify it into the cache, and open it as a head.
    let graph_ca = gantz_ca::graph_addr(&dg);
    let commit = {
        let mut registry = app.world_mut().resource_mut::<Registry>();
        registry.commit_graph(timestamp(), None, graph_ca, move || dg)
    };
    refresh_app_cache(&mut app);
    app.world_mut()
        .trigger(head::OpenEvent(gantz_ca::Head::Commit(commit)));

    // First update: `vm::sync` compiles the head's VM.
    app.update();
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<head::OpenHead>>();
    let head_entity = q.single(app.world()).expect("one open head");

    // Fire the sleep node: its task flows into `await`, which swallows the
    // evaluation; `drive_awaits` polls it in place across updates.
    app.world_mut().trigger(bevy_gantz::vm::EvalEntryEvent {
        head: head_entity,
        entrypoint: gantz_core::compile::entrypoint::push(vec![sleep.index()], 1),
        time: None,
    });

    // `sleep` forwards its (unconnected, so `'()`) input value once the
    // duration elapses: the inspect sink's state flips from `Void` to `'()`.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.update();
        let vms = app.world().non_send::<head::HeadVms>();
        let vm = vms.0.get(&head_entity).expect("head VM");
        let st = state(vm, inspect.index());
        if st != SteelVal::Void {
            assert_eq!(st, SteelVal::ListV(Default::default()));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "await result was not delivered in time",
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    // Delivery replaced the pending pair, releasing the handle from state.
    let vms = app.world().non_send::<head::HeadVms>();
    let vm = vms.0.get(&head_entity).expect("head VM");
    assert!(await_::pending_handle(&state(vm, await_n.index())).is_none());
}

/// Deleting a node reindexes its successors via swap-remove: the editor's
/// delete flow (`remove_value` + `move_value`) must carry a pending task with
/// the await node's state, and dropping an unmapped key must cancel its task.
#[test]
fn state_migration_carries_and_cancels_pending_tasks() {
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    /// Sets its flag when dropped, proving the task was released.
    struct DropFlag(Rc<Cell<bool>>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    let mut vm = Engine::new_base();
    vm.register_value(gantz_core::ROOT_STATE, SteelVal::empty_hashmap());

    // A pending await at index 2 whose task resolves on first check.
    let mut val = Some(SteelVal::IntV(11));
    let (pair, _) = pending_pair(GantzTask::poll_fn(move || val.take().map(Ok)));
    node::state::update_value(&mut vm, &[2], pair).expect("state write");
    node::state::update_value(&mut vm, &[0], SteelVal::IntV(0)).expect("state write");

    // The editor's delete flow: drop node 0's state, swap the last node (2)
    // into its index.
    node::state::remove_value(&mut vm, &[0]).expect("remove");
    node::state::move_value(&mut vm, &[2], &[0]).expect("move");

    // The pending task followed the node and still delivers.
    assert!(
        node::state::extract_value(&vm, &[2])
            .expect("read")
            .is_none()
    );
    let handle = stashed_handle(&vm, 0);
    assert_eq!(handle.check(), Some(Ok(SteelVal::IntV(11))));

    // A `remap_root` whose mapping omits the node drops its state, cancelling
    // the task.
    let flag = Rc::new(Cell::new(false));
    let guard = DropFlag(flag.clone());
    let (pair, _) = pending_pair(GantzTask::poll_fn(move || {
        let _ = &guard;
        None
    }));
    node::state::update_value(&mut vm, &[3], pair).expect("state write");
    node::state::remap_root(&mut vm, &BTreeMap::from([(0, 1)])).expect("remap");
    assert!(
        node::state::extract_value(&vm, &[3])
            .expect("read")
            .is_none()
    );
    assert!(flag.get(), "dropped state should cancel the pending task");
}

/// Regression test for in-flight awaits surviving a reindexing graph change:
/// replace the head with a child commit in which every node's index shifted
/// (a leading node removed). `migrate_vm_state` remaps the node state - and
/// with it the pending task - so the result still delivers at the new path.
#[test]
fn pending_await_survives_reindexing_replace() {
    use bevy_ecs::prelude::*;
    use bevy_gantz::{Registry, head, timestamp};
    use std::time::{Duration, Instant};

    let mut app = task_test_app();

    // Base graph: `filler, sleep(0.5) -> await -> inspect`. The filler is a
    // content-distinct sleep (the migration matcher pairs nodes by content
    // address, so duplicates would match arbitrarily).
    let mut filler_node = Sleep::default();
    filler_node.set_duration(9.0);
    let mut sleep_node = Sleep::default();
    sleep_node.set_duration(0.5);
    let mut base_dg = gantz_ca::DataGraph::default();
    let _filler = base_dg.add_node(gantz_core::data::erase_node_typed(&filler_node).unwrap());
    let sleep = base_dg.add_node(gantz_core::data::erase_node_typed(&sleep_node).unwrap());
    let await_n = base_dg.add_node(gantz_core::data::erase_node_typed(&Await).unwrap());
    let _inspect =
        base_dg.add_node(gantz_core::data::erase_node_typed(&gantz_egui::node::Inspect).unwrap());
    base_dg.add_edge(sleep, await_n, gantz_ca::Edge::from((0, 0)));
    base_dg.add_edge(await_n, _inspect, gantz_ca::Edge::from((0, 0)));

    // The child graph: identical but without the filler, so every index
    // shifts down by one.
    let mut child_dg = gantz_ca::DataGraph::default();
    let c_sleep = child_dg.add_node(gantz_core::data::erase_node_typed(&sleep_node).unwrap());
    let c_await = child_dg.add_node(gantz_core::data::erase_node_typed(&Await).unwrap());
    let c_inspect =
        child_dg.add_node(gantz_core::data::erase_node_typed(&gantz_egui::node::Inspect).unwrap());
    child_dg.add_edge(c_sleep, c_await, gantz_ca::Edge::from((0, 0)));
    child_dg.add_edge(c_await, c_inspect, gantz_ca::Edge::from((0, 0)));

    // Commit the base, open it, compile.
    let base_ca = gantz_ca::graph_addr(&base_dg);
    let base_commit = {
        let mut registry = app.world_mut().resource_mut::<Registry>();
        registry.commit_graph(timestamp(), None, base_ca, move || base_dg)
    };
    refresh_app_cache(&mut app);
    app.world_mut()
        .trigger(head::OpenEvent(gantz_ca::Head::Commit(base_commit)));
    app.update();
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<head::OpenHead>>();
    let head_entity = q.single(app.world()).expect("one open head");

    // Fire the sleep so a task is pending at the await node.
    app.world_mut().trigger(bevy_gantz::vm::EvalEntryEvent {
        head: head_entity,
        entrypoint: gantz_core::compile::entrypoint::push(vec![sleep.index()], 1),
        time: None,
    });
    app.update();
    {
        let vms = app.world().non_send::<head::HeadVms>();
        let vm = vms.0.get(&head_entity).expect("head VM");
        assert!(await_::pending_handle(&state(vm, await_n.index())).is_some());
    }

    // Replace the head with the child commit while the task is pending. The
    // chain-tracked matching remaps node state (await 2 -> 1, inspect 3 -> 2).
    let child_ca = gantz_ca::graph_addr(&child_dg);
    let child_commit = {
        let mut registry = app.world_mut().resource_mut::<Registry>();
        registry.commit_graph(timestamp(), Some(base_commit), child_ca, move || child_dg)
    };
    refresh_app_cache(&mut app);
    app.world_mut()
        .trigger(head::ReplaceEvent(gantz_ca::Head::Commit(child_commit)));

    // The result must deliver to the inspect sink at its NEW index.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.update();
        let vms = app.world().non_send::<head::HeadVms>();
        let vm = vms.0.get(&head_entity).expect("head VM");
        let st = state(vm, c_inspect.index());
        if st != SteelVal::Void {
            assert_eq!(st, SteelVal::ListV(Default::default()));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "await result was not delivered after the reindexing replace",
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
