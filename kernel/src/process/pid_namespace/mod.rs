// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use attachment::{NestedIdAttachmentInner, GLOBAL_ATTACHMENT_LOCK};
use spin::Once;

use crate::{
    events::{Events, Observer, Subject},
    prelude::*,
    thread::Thread,
};

mod attachment;
mod nested_id;

pub use attachment::{NestedIdAttachment, NestedIdAttachmentWriteGuard};
pub use nested_id::{NestedId, UniqueId};

use super::{Pgid, Pid, Process, ProcessGroup, Session, Sid};

pub struct PidNamespace {
    id: PidNsId,
    nested_level: usize,
    pid_allocator: AtomicU32,
    parent: Weak<PidNamespace>,
    children: Mutex<BTreeMap<PidNsId, Arc<PidNamespace>>>,
    pid_map: Mutex<BTreeMap<u32, Arc<NestedIdAttachmentInner>>>,
    subject: Subject<PidEvent>,
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
            pid_map: Mutex::new(BTreeMap::new()),
            subject: Subject::new(),
            is_init_proc_terminated: AtomicBool::new(false),
        }
    }

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
            pid_map: Mutex::new(BTreeMap::new()),
            subject: Subject::new(),
            is_init_proc_terminated: AtomicBool::new(false),
        });
        parent.children.lock().insert(id, child.clone());
        Ok(child)
    }

    pub fn allocate_nested_id(self: &Arc<Self>) -> NestedId {
        let _global_guard = GLOBAL_ATTACHMENT_LOCK.write();

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

        let nested_id = NestedId(Arc::new(ids));
        let nested_id_map = Arc::new(NestedIdAttachmentInner::new(nested_id.clone()));

        namespace = self.clone();

        loop {
            let id = namespace.get_current_id(&nested_id).unwrap();
            namespace.pid_map.lock().insert(id, nested_id_map.clone());

            if namespace.nested_level == 0 {
                break;
            }

            namespace = namespace.parent.upgrade().unwrap();
        }

        nested_id
    }

    pub fn dealloc_attachment(self: &Arc<Self>, nested_id: &NestedId) {
        let _global_guard = GLOBAL_ATTACHMENT_LOCK.write();

        // We should ensure that the nested id is not used by any thread or process.
        let mut namespace = self.clone();
        let Some(id) = self.get_current_id(nested_id) else {
            return;
        };

        let Some(attachment) = self.pid_map.lock().get(&id).cloned() else {
            return;
        };

        if attachment.write().has_attached() {
            return;
        }

        loop {
            let id = namespace.get_current_id(&nested_id).unwrap();
            let removed_nested_id = namespace.pid_map.lock().remove(&id).unwrap();
            debug_assert_eq!(&removed_nested_id.ids, nested_id);

            if namespace.nested_level == 0 {
                break;
            }

            namespace = namespace.parent.upgrade().unwrap();
        }
    }

    pub fn get_current_id(self: &Arc<Self>, nested_id: &NestedId) -> Option<u32> {
        let unique_id = nested_id.0.get(self.nested_level)?;

        Weak::ptr_eq(&unique_id.pid_ns, &Arc::downgrade(self)).then_some(unique_id.id)
    }

    pub fn free_namespace(self: &Arc<Self>) {
        let Some(parent) = self.parent.upgrade() else {
            return;
        };

        parent.children.lock().remove(&self.id);
    }

    pub fn get_thread(&self, id: u32) -> Option<Arc<Thread>> {
        let pid_map_guard = self.pid_map.lock();
        let nested_id = pid_map_guard.get(&id)?;
        let attach_map = nested_id.attachment.read();
        attach_map.thread.clone()
    }

    pub fn get_process(&self, id: Pid) -> Option<Arc<Process>> {
        let pid_map_guard = self.pid_map.lock();
        let nested_id_map = pid_map_guard.get(&id)?;
        let attach_map = nested_id_map.attachment.read();
        attach_map.process.clone()
    }

    pub fn get_all_processes(&self) -> Vec<Arc<Process>> {
        let pid_map_guard = self.pid_map.lock();
        pid_map_guard
            .values()
            .filter_map(|nested_id| nested_id.attachment.read().process.clone())
            .collect()
    }

    pub fn get_all_pids(&self) -> Vec<Pid> {
        self.pid_map.lock().keys().cloned().collect()
    }

    pub fn get_process_group(&self, id: Pgid) -> Option<Arc<ProcessGroup>> {
        let pid_map_guard = self.pid_map.lock();
        let nested_id = pid_map_guard.get(&id)?;
        let attach_map = nested_id.attachment.read();
        attach_map.process_group.as_ref().cloned()
    }

    pub fn get_session(&self, id: Sid) -> Option<Arc<Session>> {
        let pid_map_guard = self.pid_map.lock();
        let nested_id = pid_map_guard.get(&id)?;
        let attach_map = nested_id.attachment.read();
        attach_map.session.as_ref().cloned()
    }

    pub fn add_session(&self, id: Sid, session: Arc<Session>) {
        let pid_map_guard = self.pid_map.lock();
        let nested_id_map = pid_map_guard.get(&id).unwrap();
        let mut attach_map = nested_id_map.attachment.write();

        debug_assert!(attach_map.session.is_none());
        attach_map.session = Some(session);
    }

    pub fn get_attachment(self: &Arc<Self>, nested_id: &NestedId) -> Option<NestedIdAttachment> {
        // Hold the global lock at first
        let _guard = GLOBAL_ATTACHMENT_LOCK.read();

        let current_id = self.get_current_id(nested_id)?;
        let attachment_inner = self.pid_map.lock().get(&current_id)?.clone();

        Some(NestedIdAttachment {
            _guard,
            inner: attachment_inner,
        })
    }

    pub fn get_attachment_with_unique_id(
        self: &Arc<Self>,
        unique_id: &UniqueId,
    ) -> Option<NestedIdAttachment> {
        // Hold the global lock at first
        let _guard = GLOBAL_ATTACHMENT_LOCK.read();

        if !Weak::ptr_eq(&Arc::downgrade(self), &unique_id.pid_ns) {
            return None;
        }

        let attachment_inner = self.pid_map.lock().get(&unique_id.id)?.clone();

        Some(NestedIdAttachment {
            _guard,
            inner: attachment_inner,
        })
    }

    pub fn register_observer(&self, observer: Weak<dyn Observer<PidEvent>>) {
        self.subject.register_observer(observer, ());
    }

    pub fn unregister_observer(&self, observer: &Weak<dyn Observer<PidEvent>>) {
        self.subject.unregister_observer(observer);
    }

    pub fn set_init_proc_terminated(&self) {
        self.is_init_proc_terminated.store(true, Ordering::Relaxed);
    }

    pub fn is_init_proc_terminated(&self) -> bool {
        self.is_init_proc_terminated.load(Ordering::Relaxed)
    }
}

#[derive(Copy, Clone)]
pub enum PidEvent {
    Exit,
}

impl Events for PidEvent {}

type PidNsId = usize;
static PID_NS_ID_ALLOCATOR: AtomicUsize = AtomicUsize::new(1);
const MAX_NESTED_LEVEL: usize = 32;
pub const INIT_PROCESS_PID: Pid = 1;

static PID_ROOT_NAMESPACE: Once<Arc<PidNamespace>> = Once::new();

pub(super) fn init() {
    PID_ROOT_NAMESPACE.call_once(|| Arc::new(PidNamespace::new_root()));
}

pub fn get_root_pid_namespace() -> Arc<PidNamespace> {
    PID_ROOT_NAMESPACE.get().unwrap().clone()
}
