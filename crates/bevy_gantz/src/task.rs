//! Opaque task values for asynchronous work in gantz graphs.
//!
//! A [`GantzTask`] represents work whose result arrives at some unknown later
//! time (a network response, a DB query, a timer). Producer nodes construct
//! one (usually via [`GantzTask::spawn`]) and emit it wrapped in a
//! [`TaskHandle`] as an opaque steel value. An `await` node downstream stashes
//! the handle in its state, and a driver system polls the task each frame via
//! [`GantzTask::check`], delivering the result back into the graph with a push
//! evaluation.
//!
//! Steel values are not `Send`, so a spawned task's future must produce a
//! `Send` output. The conversion to a [`SteelVal`] happens on the main thread
//! when the task completes, via a closure that may itself capture non-`Send`
//! steel values.

use bevy_tasks::{AsyncComputeTaskPool, Task, TaskPool};
use std::{any::Any, cell::RefCell, future::Future, rc::Rc};
use steel::{
    SteelVal,
    rvals::{Custom, IntoSteelVal},
    steel_vm::engine::Engine,
};

/// A unit of asynchronous work resolving to a steel value or an error string.
///
/// Dropping a `GantzTask` cancels it: a task spawned via [`GantzTask::spawn`]
/// detaches from its pool and stops, and a [`GantzTask::poll_fn`] closure is
/// simply never called again.
pub struct GantzTask {
    kind: Kind,
}

/// A cloneable, shareable handle to an optional [`GantzTask`], suitable for
/// storage in node state or transfer along graph edges as an opaque
/// [`SteelVal`].
///
/// Cloning shares the underlying cell: steel's `FromSteelVal` for custom
/// types clones, so a handle extracted from the VM still refers to the same
/// task. The task is delivered exactly once via [`TaskHandle::take`] - the
/// first taker wins, and every other holder of the handle observes `None`.
/// In particular, wiring one task value into several `await` nodes fires
/// only one of them.
#[derive(Clone)]
pub struct TaskHandle(Rc<RefCell<Option<GantzTask>>>);

enum Kind {
    /// Work running on the async compute task pool.
    Spawned {
        task: Task<Box<dyn Any + Send>>,
        /// Converts the future's `Send` output to a steel value on the main
        /// thread. `None` once the result has been delivered.
        convert: Option<Box<dyn FnOnce(Box<dyn Any + Send>) -> Result<SteelVal, String>>>,
    },
    /// Work checked by a plain closure each frame, with no executor involved.
    Poll(Box<dyn FnMut() -> Option<Result<SteelVal, String>>>),
}

/// The name under which the [`TaskHandle`] type predicate is registered in
/// node VMs, allowing generated code to test `(gantz-task? x)`.
pub const TASK_PREDICATE: &str = "gantz-task?";

impl GantzTask {
    /// Spawn the given future on the async compute task pool.
    ///
    /// The future's output is converted to a steel value by `convert`, which
    /// runs on the main thread once the task completes and so may capture
    /// non-`Send` values (including [`SteelVal`]s).
    pub fn spawn<T, F, C>(fut: F, convert: C) -> Self
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
        C: FnOnce(T) -> Result<SteelVal, String> + 'static,
    {
        let task = AsyncComputeTaskPool::get_or_init(TaskPool::default)
            .spawn(async move { Box::new(fut.await) as Box<dyn Any + Send> });
        let convert = Box::new(move |out: Box<dyn Any + Send>| {
            let t = out.downcast::<T>().expect("spawned task output is `T`");
            convert(*t)
        });
        let kind = Kind::Spawned {
            task,
            convert: Some(convert),
        };
        Self { kind }
    }

    /// Spawn a future whose output converts directly to a steel value.
    ///
    /// See [`GantzTask::spawn`] for the threading contract.
    pub fn spawn_value<T, F>(fut: F) -> Self
    where
        T: Send + IntoSteelVal + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        Self::spawn(fut, |t| t.into_steelval().map_err(|e| e.to_string()))
    }

    /// A task backed by a closure checked once per frame by the driver,
    /// rather than a future running on an executor.
    ///
    /// Suited to work that only needs the passage of frames (e.g. timers):
    /// an executor-driven future would need to wake itself to be re-polled,
    /// busy-spinning a pool thread, whereas the driver checks this closure
    /// each update anyway.
    pub fn poll_fn<F>(f: F) -> Self
    where
        F: FnMut() -> Option<Result<SteelVal, String>> + 'static,
    {
        Self {
            kind: Kind::Poll(Box::new(f)),
        }
    }

    /// Check whether the task has resolved, returning its result if so.
    ///
    /// Returns `None` while still pending, and also on any check after the
    /// one that delivered the result.
    pub fn check(&mut self) -> Option<Result<SteelVal, String>> {
        match &mut self.kind {
            Kind::Spawned { task, convert } => {
                let out = bevy_tasks::futures::check_ready(task)?;
                let convert = convert.take()?;
                Some(convert(out))
            }
            Kind::Poll(f) => f(),
        }
    }
}

impl TaskHandle {
    /// Wrap a task in a fresh shareable handle.
    pub fn new(task: GantzTask) -> Self {
        Self(Rc::new(RefCell::new(Some(task))))
    }

    /// Take the task out of the handle, leaving `None` for every clone.
    pub fn take(&self) -> Option<GantzTask> {
        self.0.borrow_mut().take()
    }
}

impl Custom for TaskHandle {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        let state = match self.0.borrow().is_some() {
            true => "pending",
            false => "taken",
        };
        Some(Ok(format!("#<gantz-task {state}>")))
    }
}

/// Register the [`TaskHandle`] type and its [`TASK_PREDICATE`] predicate in
/// the given VM if not already present.
///
/// Guarded so that repeated registration (e.g. on every recompile) doesn't
/// leak fresh global slots.
pub fn register_task_type(vm: &mut Engine) {
    if vm.extract_value(TASK_PREDICATE).is_err() {
        vm.register_type::<TaskHandle>(TASK_PREDICATE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(val: SteelVal) -> GantzTask {
        let mut val = Some(val);
        GantzTask::poll_fn(move || val.take().map(Ok))
    }

    #[test]
    fn predicate_distinguishes_task_handles() {
        let mut vm = Engine::new_base();
        register_task_type(&mut vm);
        let handle = TaskHandle::new(ready(SteelVal::IntV(1)));
        let val = handle.into_steelval().unwrap();
        vm.register_value("t", val);
        vm.register_value("n", SteelVal::IntV(42));
        let res = vm.run(format!("({TASK_PREDICATE} t)")).unwrap();
        assert_eq!(res.last(), Some(&SteelVal::BoolV(true)));
        let res = vm.run(format!("({TASK_PREDICATE} n)")).unwrap();
        assert_eq!(res.last(), Some(&SteelVal::BoolV(false)));
    }

    #[test]
    fn clones_share_the_cell() {
        let handle = TaskHandle::new(ready(SteelVal::IntV(7)));
        let clone = handle.clone();
        let mut task = clone.take().expect("first take yields the task");
        assert!(handle.take().is_none());
        assert_eq!(task.check(), Some(Ok(SteelVal::IntV(7))));
        assert_eq!(task.check(), None);
    }

    #[test]
    fn round_trips_through_steelval() {
        use steel::rvals::FromSteelVal;
        let handle = TaskHandle::new(ready(SteelVal::IntV(3)));
        let val = handle.clone().into_steelval().unwrap();
        let extracted = TaskHandle::from_steelval(&val).unwrap();
        assert!(extracted.take().is_some());
        assert!(handle.take().is_none());
    }

    #[test]
    fn poll_fn_pends_until_ready() {
        let mut count = 0;
        let mut task = GantzTask::poll_fn(move || {
            count += 1;
            (count >= 3).then(|| Ok(SteelVal::IntV(9)))
        });
        assert_eq!(task.check(), None);
        assert_eq!(task.check(), None);
        assert_eq!(task.check(), Some(Ok(SteelVal::IntV(9))));
    }
}
