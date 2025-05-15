// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use ostd::sync::{PreemptDisabled, RcuOption, RwMutexReadGuard};

use super::{PidEvent, UniqueIdArray};
use crate::{
    events::{Observer, Subject},
    prelude::*,
    process::{Process, ProcessGroup, Session},
    thread::Thread,
};

/// A map that associates a [`UniqueIdArray`] with tasks whose IDs match the array.
///
/// It is important to note that tasks of different types can share the same ID
/// (for instance, a process and a thread might have the same ID).
/// However, two tasks of the same type cannot share the same ID
/// (for example, two threads can never have the same ID).
/// Consequently, the map can contain at most one task for each type.
pub(super) struct UniqueIdsMapInner {
    pub(super) ids: UniqueIdArray,
    subject: Subject<PidEvent>,

    // FIXME: We don't intend to achieve any synchronization with these `RcuOption`.
    // We simply want a structure that implements interior mutability, allowing us to
    // avoid using internal locks here. Synchronization is maintained via a global task list lock.
    pub(super) thread: RcuOption<Arc<Thread>>,
    pub(super) process: RcuOption<Arc<Process>>,
    pub(super) process_group: RcuOption<Arc<ProcessGroup>>,
    pub(super) session: RcuOption<Arc<Session>>,
}

impl UniqueIdsMapInner {
    /// Creates a new empty `UniqueIdsMapInner`.
    pub(super) const fn new_empty(ids: UniqueIdArray) -> Self {
        Self {
            ids,
            subject: Subject::new(),
            thread: RcuOption::new_none(),
            process: RcuOption::new_none(),
            process_group: RcuOption::new_none(),
            session: RcuOption::new_none(),
        }
    }

    /// Creates a new [`UniqueIdsMapWithTasklistGuard`] with the given `task_list_guard`.
    pub(super) fn with_task_list_guard<'a>(
        &'a self,
        task_list_guard: &'a mut TaskListGuard,
    ) -> UniqueIdsMapWithTasklistGuard<'a> {
        UniqueIdsMapWithTasklistGuard {
            inner: self,
            guard: task_list_guard,
        }
    }
}

/// A guard for the [`UniqueIdsMapInner`].
///
/// Holding this guard ensures that the associated [`UniqueIdsMapInner`]
/// exists within the PID namespaces to which it belongs.
pub struct UniqueIdsMap {
    pub(super) _guard: UidsMapReadGuard,
    pub(super) inner: Arc<UniqueIdsMapInner>,
}

impl UniqueIdsMap {
    /// Returns the associated IDs.
    pub fn ids(&self) -> &UniqueIdArray {
        &self.inner.ids
    }

    pub fn with_task_list_guard<'a>(
        &'a self,
        task_list_guard: &'a mut TaskListGuard,
    ) -> UniqueIdsMapWithTasklistGuard<'a> {
        self.inner.with_task_list_guard(task_list_guard)
    }

    pub fn register_observer(&self, observer: Weak<dyn Observer<PidEvent>>) {
        self.inner.subject.register_observer(observer, ());
    }
}

/// A guard that combines a [`UniqueIdsMap`] and a [`TaskListGuard`].
///
/// Holding this guard ensures that the task list cannot be altered concurrently,
/// allowing you to safely get, attach, or detach tasks without race conditions.
pub struct UniqueIdsMapWithTasklistGuard<'a> {
    inner: &'a UniqueIdsMapInner,
    guard: &'a mut TaskListGuard,
}

impl<'a> UniqueIdsMapWithTasklistGuard<'a> {
    pub fn new(map: &'a UniqueIdsMap, guard: &'a mut TaskListGuard) -> Self {
        Self {
            inner: &map.inner,
            guard,
        }
    }
}

impl UniqueIdsMapWithTasklistGuard<'_> {
    /// Returns the attached thread.
    pub fn attached_thread(&self) -> Option<Arc<Thread>> {
        self.inner
            .thread
            .read_with(self.guard)
            .map(|thread| thread.clone())
    }

    /// Returns the attached process.
    pub fn attached_process(&self) -> Option<Arc<Process>> {
        self.inner
            .process
            .read_with(self.guard)
            .map(|process| process.clone())
    }

    /// Returns the attached process group.
    pub fn attached_process_group(&self) -> Option<Arc<ProcessGroup>> {
        self.inner
            .process_group
            .read_with(self.guard)
            .map(|process_group| process_group.clone())
    }

    /// Returns the attached session.
    pub fn attached_session(&self) -> Option<Arc<Session>> {
        self.inner
            .session
            .read_with(self.guard)
            .map(|session| session.clone())
    }

    /// Attaches a thread to the [`UniqueIdsMap`].
    ///
    /// After attachment, the thread will be visible to other processes
    /// in the same or ancestor PID namespaces.
    pub fn attach_thread(&mut self, thread: Arc<Thread>) {
        debug_assert!(self.inner.thread.read_with(self.guard).is_none());
        self.inner.thread.update(Some(thread));
    }

    /// Attaches a process to the [`UniqueIdsMap`].
    pub fn attach_process(&mut self, process: Arc<Process>) {
        debug_assert!(self.inner.process.read_with(self.guard).is_none());
        self.inner.process.update(Some(process));
    }

    /// Attaches a process group to the [`UniqueIdsMap`].
    pub fn attach_process_group(&mut self, process_group: Arc<ProcessGroup>) {
        debug_assert!(self.inner.process_group.read_with(self.guard).is_none());
        self.inner.process_group.update(Some(process_group));
    }

    /// Attaches a session to the [`UniqueIdsMap`].
    pub fn attach_session(&mut self, session: Arc<Session>) {
        debug_assert!(self.inner.session.read_with(self.guard).is_none());
        self.inner.session.update(Some(session));
    }

    /// Detaches a thread from the [`UniqueIdsMap`].
    ///
    /// After detachment, the thread will be invisible to other processes in the same or ancestor PID namespaces.
    pub fn detach_thread(&mut self) {
        debug_assert!(self.inner.thread.read_with(self.guard).is_some());
        self.inner.thread.update(None);
    }

    /// Detaches a process from the [`UniqueIdsMap`] and notifies observers of its exit.
    pub(super) fn detach_process(&mut self) {
        debug_assert!(self.inner.process.read_with(self.guard).is_some());
        self.inner.process.update(None);
        self.inner.subject.notify_observers(&PidEvent::Exit);
    }

    /// Detaches a process group from the [`UniqueIdsMap`].
    pub(super) fn detach_process_group(&mut self) {
        debug_assert!(self.inner.process_group.read_with(self.guard).is_some());
        self.inner.process_group.update(None);
    }

    /// Detaches a session from the [`UniqueIdsMap`].
    pub(super) fn detach_session(&mut self) {
        debug_assert!(self.inner.session.read_with(self.guard).is_some());
        self.inner.session.update(None);
    }

    /// Checks if there is any task attached to the [`UniqueIdsMap`].
    pub(super) fn has_attached(&self) -> bool {
        self.inner.thread.read_with(self.guard).is_some()
            || self.inner.process.read_with(self.guard).is_some()
            || self.inner.process_group.read_with(self.guard).is_some()
            || self.inner.session.read_with(self.guard).is_some()
    }
}

pub struct TaskList(PhantomData<()>);

/// The global lock for protecting task operations such as attach, detach, and get.
pub static TASK_LIST_LOCK: SpinLock<TaskList> = SpinLock::new(TaskList(PhantomData));
pub(super) type TaskListGuard = SpinLockGuard<'static, TaskList, PreemptDisabled>;

pub(super) struct UidsMap(PhantomData<()>);

/// The global lock for safeguarding operations like get, add, and remove of [`UniqueIdsMap`] within namespaces.
pub(super) static UIDS_MAP_LOCK: RwMutex<UidsMap> = RwMutex::new(UidsMap(PhantomData));
pub(super) type UidsMapReadGuard = RwMutexReadGuard<'static, UidsMap>;
