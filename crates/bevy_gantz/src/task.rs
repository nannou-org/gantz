//! Opaque task values for asynchronous work in gantz graphs.
//!
//! A [`GantzTask`] represents work whose result arrives at some later time.
//! Examples are a network response, a DB query or a timer. Producer nodes
//! construct one, usually via [`GantzTask::spawn`], and emit it wrapped in a
//! [`TaskHandle`] as an opaque steel value. An `await` node downstream stores
//! the handle in its state. A driver system polls it in place each frame via
//! [`TaskHandle::check`] and delivers the result back into the graph with a
//! push evaluation. The pending task lives inside node state, so the
//! state-migration machinery carries in-flight work wherever its node goes.
//!
//! Steel values are not `Send`, so a spawned task's future must produce a
//! `Send` output. The conversion to a [`SteelVal`] happens on the main thread
//! when the task completes. The conversion closure may capture non-`Send`
//! steel values.

use bevy_tasks::{AsyncComputeTaskPool, Task, TaskPool};
use std::{any::Any, cell::RefCell, future::Future, rc::Rc};
use steel::{
    SteelVal,
    rvals::{Custom, FromSteelVal, IntoSteelVal},
    steel_vm::engine::Engine,
    steel_vm::register_fn::RegisterFn,
};

/// A unit of asynchronous work resolving to a steel value or an error string.
///
/// Dropping a `GantzTask` cancels it. A task spawned via [`GantzTask::spawn`]
/// detaches from its pool and stops. A [`GantzTask::poll_fn`] closure is
/// never called again.
pub struct GantzTask {
    kind: Kind,
}

/// A cloneable, shareable handle to an optional [`GantzTask`], suitable for
/// storage in node state or transfer along graph edges as an opaque
/// [`SteelVal`].
///
/// Cloning shares the underlying cell. Steel's `FromSteelVal` for custom
/// types clones, so a handle extracted from the VM still refers to the same
/// task. The result is delivered exactly once. [`TaskHandle::check`] consumes
/// the result on completion. When several holders share the cell, only the
/// first holder checked after completion delivers. The rest observe `None`.
/// [`TaskHandle::take`] and [`TaskHandle::cancel`] remove the task from every
/// clone at once.
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

/// The name of the [`TaskHandle`] type predicate registered in node VMs.
/// Generated code tests `(gantz-task? x)`.
pub const TASK_PREDICATE: &str = "gantz-task?";

/// The name of the registered fn that cancels a pending task. Generated code
/// calls `(gantz-task-cancel! x)`. `x` is a task handle or the `await` node's
/// state pair, a list whose second element is a handle.
pub const TASK_CANCEL_FN: &str = "gantz-task-cancel!";

impl GantzTask {
    /// Spawn the given future on the async compute task pool.
    ///
    /// `convert` turns the future's output into a steel value. It runs on the
    /// main thread once the task completes, so it may capture non-`Send`
    /// values such as [`SteelVal`]s.
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

    /// A task backed by a closure that the driver checks once per frame.
    ///
    /// Suited to work such as timers that only needs the passage of frames.
    /// An executor-driven future would need to wake itself to be re-polled
    /// and would busy-spin a pool thread. The driver checks this closure each
    /// update anyway.
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

    /// Check the contained task in place, leaving it in the handle.
    ///
    /// Returns `None` while pending, after the result has been delivered, or
    /// when the handle was taken or cancelled.
    pub fn check(&self) -> Option<Result<SteelVal, String>> {
        self.0.borrow_mut().as_mut()?.check()
    }

    /// Drop the contained task, cancelling it for every clone.
    ///
    /// Returns whether a task was present to cancel.
    pub fn cancel(&self) -> bool {
        self.0.borrow_mut().take().is_some()
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

/// Register the [`TaskHandle`] type, its [`TASK_PREDICATE`] predicate and the
/// [`TASK_CANCEL_FN`] fn in the given VM if not already present.
///
/// The guard stops repeated registration on every recompile from leaking
/// fresh global slots.
pub fn register_task_type(vm: &mut Engine) {
    if vm.extract_value(TASK_PREDICATE).is_err() {
        vm.register_type::<TaskHandle>(TASK_PREDICATE);
        vm.register_fn(TASK_CANCEL_FN, cancel_task_value);
    }
}

/// Cancel the task in `val`. See [`TASK_CANCEL_FN`] for the accepted forms.
/// No-op on anything else.
///
/// Returns whether a task was present to cancel. Cancellation must be
/// explicit. Steel heap-boxes `set!`-mutated state, so a superseded value may
/// linger until its slot is recycled.
fn cancel_task_value(val: SteelVal) -> bool {
    task_handle_of(&val).is_some_and(|handle| handle.cancel())
}

/// The task handle in `val`, either `val` itself or the second element of the
/// `await` node's `(list arm payload)` state pair.
fn task_handle_of(val: &SteelVal) -> Option<TaskHandle> {
    if let Ok(handle) = TaskHandle::from_steelval(val) {
        return Some(handle);
    }
    let SteelVal::ListV(list) = val else {
        return None;
    };
    TaskHandle::from_steelval(list.iter().nth(1)?).ok()
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

    /// Sets its flag when dropped, proving a task was actually released.
    struct DropFlag(Rc<std::cell::Cell<bool>>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    /// A never-resolving task whose drop sets the given flag.
    fn flagged_task(flag: &Rc<std::cell::Cell<bool>>) -> GantzTask {
        let guard = DropFlag(flag.clone());
        GantzTask::poll_fn(move || {
            let _ = &guard;
            None
        })
    }

    #[test]
    fn check_in_place_leaves_the_task_until_delivery() {
        let mut count = 0;
        let task = GantzTask::poll_fn(move || {
            count += 1;
            (count == 2).then(|| Ok(SteelVal::IntV(4)))
        });
        let handle = TaskHandle::new(task);
        let clone = handle.clone();
        assert_eq!(clone.check(), None);
        assert_eq!(handle.check(), Some(Ok(SteelVal::IntV(4))));
        assert_eq!(clone.check(), None);
    }

    #[test]
    fn cancel_drops_the_task() {
        let flag = Rc::new(std::cell::Cell::new(false));
        let handle = TaskHandle::new(flagged_task(&flag));
        assert!(!flag.get());
        assert!(handle.cancel());
        assert!(flag.get());
        assert!(!handle.cancel());
    }

    #[test]
    fn cancel_fn_cancels_pending_pairs() {
        let mut vm = Engine::new_base();
        register_task_type(&mut vm);
        let flag = Rc::new(std::cell::Cell::new(false));
        let handle = TaskHandle::new(flagged_task(&flag));
        let pair = SteelVal::ListV(
            [SteelVal::IntV(2), handle.into_steelval().unwrap()]
                .into_iter()
                .collect(),
        );
        vm.register_value("p", pair);
        vm.register_value("n", SteelVal::IntV(1));
        let res = vm.run(format!("({TASK_CANCEL_FN} n)")).unwrap();
        assert_eq!(res.last(), Some(&SteelVal::BoolV(false)));
        assert!(!flag.get());
        let res = vm.run(format!("({TASK_CANCEL_FN} p)")).unwrap();
        assert_eq!(res.last(), Some(&SteelVal::BoolV(true)));
        assert!(flag.get());
    }
}
