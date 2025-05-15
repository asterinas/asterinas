// SPDX-License-Identifier: MPL-2.0

//! This module defines the PID namespace.
//!
//! PID namespaces are used to isolate threads, processes, process groups and sessions
//! (we will call these four as `task`s ).
//! Each task belongs to a unique PID namespace.
//! Additionally, each PID namespace has its own PID allocator,
//! which means that tasks in different PID namespaces might have the same PID.
//!
//! # Tree Structure
//!
//! All PID namespaces form a hierarchical tree structure,
//! with the init PID namespace as the root.
//! When the system boots, only a single PID namespace exists,
//! known as the init PID namespace.
//! Upon the creation of a new PID namespace,
//! it becomes a child of the current PID namespace.
//! This process resembles the creation of processes,
//! where a newly cloned process becomes a child of the existing process.
//! Linux imposes a maximum depth limit of 32 for this tree.
//!
//! # Isolation
//!
//! A task is only visible within its current PID namespace
//! and all its ancestor namespaces.
//! Being "visible" means that the process has a unique PID in these namespaces,
//! allowing it to be the target for system calls such as kill and waitpid,
//! and to appear in the procfs of these namespaces.
//!

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use spin::Once;
use unique_id::{TaskId, UniqueId};
use unique_ids_map::{UniqueIdsMapInner, UIDS_MAP_LOCK};

use crate::{
    events::Events,
    prelude::*,
    thread::{Thread, Tid},
};

mod maps_of_process;
mod unique_id;
mod unique_ids_map;

pub use maps_of_process::MapsOfProcess;
pub use unique_id::UniqueIdArray;
pub use unique_ids_map::{UniqueIdsMap, UniqueIdsMapWithTasklistGuard, TASK_LIST_LOCK};

use super::{Pgid, Pid, Process, ProcessGroup, Session, Sid};

/// The PID namespace.
pub struct PidNamespace {
    id: usize,
    nested_level: usize,
    pid_allocator: AtomicU32,
    parent: Weak<PidNamespace>,
    children: Mutex<BTreeMap<usize, Arc<PidNamespace>>>,
    uids_maps: Mutex<BTreeMap<TaskId, Arc<UniqueIdsMapInner>>>,
    is_init_proc_terminated: AtomicBool,
}

impl PidNamespace {
    /// Creates a new root namespace.
    pub fn new_root() -> Self {
        let id = PID_NS_ID_ALLOCATOR.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            nested_level: 0,
            pid_allocator: AtomicU32::new(1),
            parent: Weak::new(),
            children: Mutex::new(BTreeMap::new()),
            uids_maps: Mutex::new(BTreeMap::new()),
            is_init_proc_terminated: AtomicBool::new(false),
        }
    }

    /// Creates a new child namespace.
    pub fn new_child(parent: &Arc<Self>) -> Result<Arc<Self>> {
        let nested_level = parent.nested_level + 1;
        if nested_level >= MAX_NESTED_LEVEL {
            return_errno_with_message!(
                Errno::EINVAL,
                "the namespace nested level has reached its limit"
            );
        }

        let id = PID_NS_ID_ALLOCATOR.fetch_add(1, Ordering::Relaxed);
        let child = Arc::new(Self {
            id,
            nested_level,
            pid_allocator: AtomicU32::new(1),
            parent: Arc::downgrade(parent),
            children: Mutex::new(BTreeMap::new()),
            uids_maps: Mutex::new(BTreeMap::new()),
            is_init_proc_terminated: AtomicBool::new(false),
        });
        parent.children.lock().insert(id, child.clone());
        Ok(child)
    }

    /// Allocates a [`UniqueIdArray`] from the namespace and all ancestor PID namespaces.
    ///
    /// An empty [`UniqueIdsMap`] will also be added to all these namespaces.
    pub fn allocate_unique_ids(self: &Arc<Self>) -> UniqueIdArray {
        let _global_guard = UIDS_MAP_LOCK.write();

        let mut namespace = self.clone();
        let mut ids = VecDeque::new();

        loop {
            let id = namespace.pid_allocator.fetch_add(1, Ordering::Relaxed);
            ids.push_front(UniqueId {
                id,
                pid_ns: Arc::downgrade(&namespace),
            });

            if namespace.nested_level == 0 {
                break;
            }

            namespace = namespace.parent.upgrade().unwrap();
        }

        let unique_ids = UniqueIdArray(Arc::new(ids));
        let unique_ids_map = Arc::new(UniqueIdsMapInner::new_empty(unique_ids.clone()));

        namespace = self.clone();

        loop {
            let id = namespace.get_current_id(&unique_ids).unwrap();
            namespace
                .uids_maps
                .lock()
                .insert(id, unique_ids_map.clone());

            if namespace.nested_level == 0 {
                break;
            }

            namespace = namespace.parent.upgrade().unwrap();
        }

        unique_ids
    }

    /// Gets the `TaskId` of the `unique_ids` in this PID namespace.
    ///
    /// It the `unique_ids` is not visible in this namespace, this method will return `None`.
    pub fn get_current_id(self: &Arc<Self>, unique_ids: &UniqueIdArray) -> Option<TaskId> {
        let unique_id = unique_ids.0.get(self.nested_level)?;

        Weak::ptr_eq(&unique_id.pid_ns, &Arc::downgrade(self)).then_some(unique_id.id)
    }

    pub fn get_thread(&self, id: Tid) -> Option<Arc<Thread>> {
        let pid_map_guard = self.uids_maps.lock();
        let unique_ids_map = pid_map_guard.get(&id)?;
        unique_ids_map
            .with_task_list_guard(&mut TASK_LIST_LOCK.lock())
            .attached_thread()
    }

    pub fn get_process(&self, id: Pid) -> Option<Arc<Process>> {
        let pid_map_guard = self.uids_maps.lock();
        let unique_ids_map = pid_map_guard.get(&id)?;
        unique_ids_map
            .with_task_list_guard(&mut TASK_LIST_LOCK.lock())
            .attached_process()
    }

    pub fn get_all_processes(&self) -> Vec<Arc<Process>> {
        let pid_map_guard = self.uids_maps.lock();
        let mut task_list_guard = TASK_LIST_LOCK.lock();
        pid_map_guard
            .values()
            .filter_map(|unique_ids_map| {
                unique_ids_map
                    .with_task_list_guard(&mut task_list_guard)
                    .attached_process()
            })
            .collect()
    }

    pub fn get_process_group(&self, id: Pgid) -> Option<Arc<ProcessGroup>> {
        let pid_map_guard = self.uids_maps.lock();
        let unique_ids_map = pid_map_guard.get(&id)?;
        unique_ids_map
            .with_task_list_guard(&mut TASK_LIST_LOCK.lock())
            .attached_process_group()
    }

    pub fn get_session(&self, id: Sid) -> Option<Arc<Session>> {
        let pid_map_guard = self.uids_maps.lock();
        let unique_ids_map = pid_map_guard.get(&id)?;
        unique_ids_map
            .with_task_list_guard(&mut TASK_LIST_LOCK.lock())
            .attached_session()
    }

    pub fn get_map_by_ids(self: &Arc<Self>, unique_ids: &UniqueIdArray) -> Option<UniqueIdsMap> {
        // Hold the global lock at first
        let _guard = UIDS_MAP_LOCK.read();

        let current_id = self.get_current_id(unique_ids)?;
        let map = self.uids_maps.lock().get(&current_id)?.clone();

        Some(UniqueIdsMap { _guard, inner: map })
    }

    pub fn get_map_by_id(self: &Arc<Self>, id: TaskId) -> Option<UniqueIdsMap> {
        // Hold the global lock at first
        let _guard = UIDS_MAP_LOCK.read();

        let map = self.uids_maps.lock().get(&id)?.clone();

        Some(UniqueIdsMap { _guard, inner: map })
    }

    /// Marks the init process of the PID namespace as terminated.
    pub fn set_init_proc_terminated(&self) {
        self.is_init_proc_terminated.store(true, Ordering::Relaxed);
    }

    /// Checks whether the init process of the PID namespace has been terminated.
    pub fn is_init_proc_terminated(&self) -> bool {
        self.is_init_proc_terminated.load(Ordering::Relaxed)
    }

    /// Returns the last allocated `TaskId`.
    pub fn last_allocated_id(&self) -> TaskId {
        self.pid_allocator.load(Ordering::Relaxed) - 1
    }
}

/// Deallocates the [`UniqueIdsMap`] associated with the `unique_ids`
/// from all namespaces it belongs to,
/// if the `unique_ids` have no attached tasks.
pub fn dealloc_unique_ids(unique_ids: &UniqueIdArray) {
    let _global_guard = UIDS_MAP_LOCK.write();

    let (mut namespace, id) = {
        let unique_id = unique_ids.0.back().unwrap();
        let Some(pid_ns) = unique_id.pid_ns.upgrade() else {
            return;
        };
        (pid_ns, unique_id.id)
    };

    // Check if the `UniqueIdArray` is used by any tasks.
    let uids_maps_guard = namespace.uids_maps.lock();
    let Some(unique_ids_map) = uids_maps_guard.get(&id) else {
        return;
    };
    if unique_ids_map
        .with_task_list_guard(&mut TASK_LIST_LOCK.lock())
        .has_attached()
    {
        return;
    }
    drop(uids_maps_guard);

    loop {
        let id = namespace.get_current_id(unique_ids).unwrap();
        let mut uid_maps = namespace.uids_maps.lock();
        let removed_map = uid_maps.remove(&id).unwrap();
        debug_assert_eq!(&removed_map.ids, unique_ids);

        if namespace.nested_level == 0 {
            break;
        }

        let parent = namespace.parent.upgrade().unwrap();

        if uid_maps.is_empty() && namespace.children.lock().is_empty() {
            parent.children.lock().remove(&namespace.id);
        }

        drop(uid_maps);

        namespace = parent;
    }
}

#[derive(Copy, Clone)]
pub enum PidEvent {
    Exit,
}

impl Events for PidEvent {}

static PID_NS_ID_ALLOCATOR: AtomicUsize = AtomicUsize::new(1);
const MAX_NESTED_LEVEL: usize = 32;
pub const INIT_PROCESS_PID: Pid = 1;

static PID_INIT_NAMESPACE: Once<Arc<PidNamespace>> = Once::new();

pub(super) fn init() {
    PID_INIT_NAMESPACE.call_once(|| Arc::new(PidNamespace::new_root()));
}

pub fn get_init_pid_namespace() -> Arc<PidNamespace> {
    PID_INIT_NAMESPACE.get().unwrap().clone()
}
