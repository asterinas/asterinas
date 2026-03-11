// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use spin::Once;

use crate::{
    context::Context,
    fs::pseudofs::{NsCommonOps, NsType, StashedDentry},
    prelude::*,
    process::{
        Pgid, Pid, Process, ProcessGroup, Session, Sid, UserNamespace, posix_thread::AsPosixThread,
    },
    thread::{AsThread, Thread, Tid},
};

pub const PID_NS_LEVEL_LIMIT: u32 = 32;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KernelId(u64);

impl KernelId {
    #[expect(dead_code)]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

pub type KernelPid = KernelId;
pub type KernelTid = KernelId;

pub struct KernelIdAllocator(AtomicU64);

impl KernelIdAllocator {
    const fn new() -> Self {
        Self(AtomicU64::new(1))
    }

    pub fn alloc(&self) -> Result<KernelId> {
        let id = self
            .0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_add(1)
            })
            .map_err(|_| Error::with_message(Errno::EAGAIN, "kernel ids are exhausted"))?;
        Ok(KernelId(id))
    }

    pub fn last_allocated(&self) -> u64 {
        self.0.load(Ordering::SeqCst).saturating_sub(1)
    }
}

pub fn kernel_id_allocator() -> &'static KernelIdAllocator {
    static ALLOCATOR: KernelIdAllocator = KernelIdAllocator::new();
    &ALLOCATOR
}

#[expect(dead_code)]
pub fn pid_ns_graph_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PidNsState {
    PendingInit = 0,
    Alive = 1,
    Dying = 2,
}

pub struct AtomicPidNsState(AtomicU8);

impl AtomicPidNsState {
    pub const fn new(state: PidNsState) -> Self {
        Self(AtomicU8::new(state as u8))
    }

    pub fn load(&self) -> PidNsState {
        match self.0.load(Ordering::Acquire) {
            0 => PidNsState::PendingInit,
            1 => PidNsState::Alive,
            2 => PidNsState::Dying,
            _ => unreachable!(),
        }
    }

    pub fn store(&self, state: PidNsState) {
        self.0.store(state as u8, Ordering::Release);
    }
}

pub struct PidNamespace {
    parent: Option<Arc<PidNamespace>>,
    level: u32,
    owner_user_ns: Arc<UserNamespace>,
    state: AtomicPidNsState,
    pending_init_lock: Mutex<()>,
    allocator: AtomicU32,
    visible_table: Mutex<NsVisiblePidTable>,
    child_reaper: Mutex<Weak<Process>>,
    stashed_dentry: StashedDentry,
}

pub struct NsVisiblePidTable {
    entries: BTreeMap<u32, Arc<PidEntry>>,
}

pub struct PidEntry {
    inner: Mutex<PidEntryInner>,
}

struct PidEntryInner {
    thread: Weak<Thread>,
    process: Weak<Process>,
    process_group: Weak<ProcessGroup>,
    session: Weak<Session>,
}

#[derive(Clone)]
pub struct PidLink {
    ns: Arc<PidNamespace>,
    nr: u32,
}

#[derive(Clone)]
pub struct PidChain {
    numbers: Box<[PidLink]>,
}

#[derive(Clone)]
pub enum PidNsForChildren {
    SameAsActive,
    Target(Arc<PidNamespace>),
}

impl PidEntry {
    fn new() -> Self {
        Self {
            inner: Mutex::new(PidEntryInner {
                thread: Weak::new(),
                process: Weak::new(),
                process_group: Weak::new(),
                session: Weak::new(),
            }),
        }
    }

    fn thread(&self) -> Option<Arc<Thread>> {
        self.inner.lock().thread.upgrade()
    }

    fn process(&self) -> Option<Arc<Process>> {
        self.inner.lock().process.upgrade()
    }

    fn process_group(&self) -> Option<Arc<ProcessGroup>> {
        self.inner.lock().process_group.upgrade()
    }

    fn set_thread(&self, thread: &Arc<Thread>) {
        self.inner.lock().thread = Arc::downgrade(thread);
    }

    fn clear_thread(&self) {
        self.inner.lock().thread = Weak::new();
    }

    fn set_process(&self, process: &Arc<Process>) {
        self.inner.lock().process = Arc::downgrade(process);
    }

    fn clear_process(&self) {
        self.inner.lock().process = Weak::new();
    }

    fn set_process_group(&self, process_group: &Arc<ProcessGroup>) {
        self.inner.lock().process_group = Arc::downgrade(process_group);
    }

    fn clear_process_group(&self) {
        self.inner.lock().process_group = Weak::new();
    }

    fn set_session(&self, session: &Arc<Session>) {
        self.inner.lock().session = Arc::downgrade(session);
    }

    fn clear_session(&self) {
        self.inner.lock().session = Weak::new();
    }

    fn is_empty(&self) -> bool {
        let inner = self.inner.lock();
        inner.thread.strong_count() == 0
            && inner.process.strong_count() == 0
            && inner.process_group.strong_count() == 0
            && inner.session.strong_count() == 0
    }
}

impl NsVisiblePidTable {
    const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    fn get_or_create_entry(&mut self, nr: u32) -> Arc<PidEntry> {
        self.entries
            .entry(nr)
            .or_insert_with(|| Arc::new(PidEntry::new()))
            .clone()
    }

    fn try_remove_entry(&mut self, nr: u32) {
        if let Some(entry) = self.entries.get(&nr)
            && entry.is_empty()
        {
            self.entries.remove(&nr);
        }
    }

    fn insert_thread(&mut self, nr: Tid, thread: &Arc<Thread>) {
        self.get_or_create_entry(nr).set_thread(thread);
    }

    fn remove_thread(&mut self, nr: Tid) {
        if let Some(entry) = self.entries.get(&nr) {
            entry.clear_thread();
        }
        self.try_remove_entry(nr);
    }

    fn insert_process(&mut self, nr: Pid, process: &Arc<Process>) {
        self.get_or_create_entry(nr).set_process(process);
    }

    fn remove_process(&mut self, nr: Pid) {
        if let Some(entry) = self.entries.get(&nr) {
            entry.clear_process();
        }
        self.try_remove_entry(nr);
    }

    fn insert_process_group(&mut self, nr: Pgid, process_group: &Arc<ProcessGroup>) {
        self.get_or_create_entry(nr)
            .set_process_group(process_group);
    }

    fn remove_process_group(&mut self, nr: Pgid) {
        if let Some(entry) = self.entries.get(&nr) {
            entry.clear_process_group();
        }
        self.try_remove_entry(nr);
    }

    fn insert_session(&mut self, nr: Sid, session: &Arc<Session>) {
        self.get_or_create_entry(nr).set_session(session);
    }

    fn remove_session(&mut self, nr: Sid) {
        if let Some(entry) = self.entries.get(&nr) {
            entry.clear_session();
        }
        self.try_remove_entry(nr);
    }
}

impl PidNamespace {
    pub fn get_init_singleton() -> &'static Arc<Self> {
        static INIT: Once<Arc<PidNamespace>> = Once::new();
        INIT.call_once(|| {
            Arc::new(Self {
                parent: None,
                level: 0,
                owner_user_ns: UserNamespace::get_init_singleton().clone(),
                state: AtomicPidNsState::new(PidNsState::Alive),
                pending_init_lock: Mutex::new(()),
                allocator: AtomicU32::new(2),
                visible_table: Mutex::new(NsVisiblePidTable::new()),
                child_reaper: Mutex::new(Weak::new()),
                stashed_dentry: StashedDentry::new(),
            })
        })
    }

    pub fn new_child(
        parent: Arc<PidNamespace>,
        owner_user_ns: Arc<UserNamespace>,
    ) -> Result<Arc<Self>> {
        let next_depth = parent.level + 2;
        if next_depth > PID_NS_LEVEL_LIMIT {
            return_errno_with_message!(
                Errno::EINVAL,
                "the pid namespace nesting limit is exceeded"
            );
        }

        Ok(Arc::new(Self {
            parent: Some(parent.clone()),
            level: parent.level + 1,
            owner_user_ns,
            state: AtomicPidNsState::new(PidNsState::PendingInit),
            pending_init_lock: Mutex::new(()),
            allocator: AtomicU32::new(1),
            visible_table: Mutex::new(NsVisiblePidTable::new()),
            child_reaper: Mutex::new(Weak::new()),
            stashed_dentry: StashedDentry::new(),
        }))
    }

    #[expect(dead_code)]
    pub fn parent_ns(&self) -> Option<&Arc<PidNamespace>> {
        self.parent.as_ref()
    }

    #[expect(dead_code)]
    pub fn level(&self) -> u32 {
        self.level
    }

    pub fn state(&self) -> PidNsState {
        self.state.load()
    }

    pub fn set_state(&self, state: PidNsState) {
        self.state.store(state);
    }

    pub fn pending_init_lock(&self) -> &Mutex<()> {
        &self.pending_init_lock
    }

    #[expect(dead_code)]
    pub fn child_reaper(&self) -> Option<Arc<Process>> {
        self.child_reaper.lock().upgrade()
    }

    pub fn set_child_reaper(&self, process: &Arc<Process>) {
        *self.child_reaper.lock() = Arc::downgrade(process);
    }

    pub fn is_same_or_ancestor_of(self: &Arc<Self>, other: &Arc<Self>) -> bool {
        let mut current = Some(other.clone());
        while let Some(ns) = current {
            if Arc::ptr_eq(self, &ns) {
                return true;
            }
            current = ns.parent.clone();
        }
        false
    }

    #[expect(dead_code)]
    pub fn is_same_or_descendant_of(self: &Arc<Self>, other: &Arc<Self>) -> bool {
        other.is_same_or_ancestor_of(self)
    }

    pub fn ancestor_chain(self: &Arc<Self>) -> Box<[Arc<PidNamespace>]> {
        let mut chain = Vec::new();
        let mut current = Some(self.clone());
        while let Some(ns) = current {
            chain.push(ns.clone());
            current = ns.parent.clone();
        }
        chain.reverse();
        chain.into_boxed_slice()
    }

    pub fn alloc_visible_id(&self) -> Result<u32> {
        let next = self
            .allocator
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_add(1)
            })
            .map_err(|_| Error::with_message(Errno::EAGAIN, "pid space is exhausted"))?;
        Ok(next)
    }

    pub fn alloc_chain(self: &Arc<Self>) -> Result<PidChain> {
        let chain = self.ancestor_chain();
        let mut numbers = Vec::with_capacity(chain.len());
        for ns in chain {
            let nr = ns.alloc_visible_id()?;
            numbers.push(PidLink { ns, nr });
        }
        Ok(PidChain::from_links(numbers))
    }

    pub fn lookup_thread(&self, tid: Tid) -> Option<Arc<Thread>> {
        self.visible_table
            .lock()
            .entries
            .get(&tid)
            .and_then(|entry| entry.thread())
    }

    pub fn lookup_process(&self, pid: Pid) -> Option<Arc<Process>> {
        self.visible_table
            .lock()
            .entries
            .get(&pid)
            .and_then(|entry| entry.process())
    }

    pub fn lookup_process_group(&self, pgid: Pgid) -> Option<Arc<ProcessGroup>> {
        self.visible_table
            .lock()
            .entries
            .get(&pgid)
            .and_then(|entry| entry.process_group())
    }

    pub fn contains_process_group(&self, pgid: Pgid) -> bool {
        self.lookup_process_group(pgid).is_some()
    }

    pub fn visible_processes(&self) -> Vec<Arc<Process>> {
        self.visible_table
            .lock()
            .entries
            .values()
            .filter_map(|entry| entry.process())
            .collect()
    }

    pub fn visible_threads(&self) -> Vec<Arc<Thread>> {
        self.visible_table
            .lock()
            .entries
            .values()
            .filter_map(|entry| entry.thread())
            .collect()
    }

    pub fn visible_process_count(&self) -> usize {
        self.visible_processes().len()
    }

    pub fn insert_process_across_namespaces(process: Arc<Process>) {
        for link in process.pid_chain().links() {
            link.ns().insert_process_chain(&process);
        }
    }

    pub fn remove_process_across_namespaces(process: &Process) {
        for link in process.pid_chain().links() {
            link.ns().remove_process_chain(process);
        }
    }

    pub fn insert_thread_across_namespaces(thread: Arc<Thread>) {
        let posix_thread = thread.as_posix_thread().unwrap();
        let tid_chain = posix_thread.tid_chain().clone();
        for link in tid_chain.links() {
            link.ns().insert_thread_chain(&thread, &tid_chain);
        }
    }

    pub fn remove_thread_across_namespaces(tid_chain: &PidChain) {
        for link in tid_chain.links() {
            link.ns().remove_thread_chain(tid_chain);
        }
    }

    pub fn insert_process_group_across_namespaces(process_group: Arc<ProcessGroup>) {
        for link in process_group.pgid_chain().links() {
            link.ns()
                .insert_process_group_chain(&process_group, process_group.pgid_chain());
        }
    }

    pub fn remove_process_group_across_namespaces(process_group: &ProcessGroup) {
        for link in process_group.pgid_chain().links() {
            link.ns()
                .remove_process_group_chain(process_group.pgid_chain());
        }
    }

    pub fn insert_session_across_namespaces(session: Arc<Session>) {
        for link in session.sid_chain().links() {
            link.ns()
                .insert_session_chain(&session, session.sid_chain());
        }
    }

    pub fn remove_session_across_namespaces(session: &Session) {
        for link in session.sid_chain().links() {
            link.ns().remove_session_chain(session.sid_chain());
        }
    }

    pub fn insert_process_chain(&self, process: &Arc<Process>) {
        let mut table = self.visible_table.lock();
        for link in process.pid_chain().links() {
            if core::ptr::eq(Arc::as_ptr(&link.ns), self) {
                table.insert_process(link.nr, process);
            }
        }
    }

    pub fn insert_thread_chain(&self, thread: &Arc<Thread>, tid_chain: &PidChain) {
        let mut table = self.visible_table.lock();
        for link in tid_chain.links() {
            if core::ptr::eq(Arc::as_ptr(&link.ns), self) {
                table.insert_thread(link.nr, thread);
            }
        }
    }

    pub fn remove_process_chain(&self, process: &Process) {
        let mut table = self.visible_table.lock();
        for link in process.pid_chain().links() {
            if core::ptr::eq(Arc::as_ptr(&link.ns), self) {
                table.remove_process(link.nr);
            }
        }
    }

    pub fn remove_thread_chain(&self, tid_chain: &PidChain) {
        let mut table = self.visible_table.lock();
        for link in tid_chain.links() {
            if core::ptr::eq(Arc::as_ptr(&link.ns), self) {
                table.remove_thread(link.nr);
            }
        }
    }

    pub fn insert_process_group_chain(
        &self,
        process_group: &Arc<ProcessGroup>,
        pgid_chain: &PidChain,
    ) {
        let mut table = self.visible_table.lock();
        for link in pgid_chain.links() {
            if core::ptr::eq(Arc::as_ptr(&link.ns), self) {
                table.insert_process_group(link.nr, process_group);
            }
        }
    }

    pub fn remove_process_group_chain(&self, pgid_chain: &PidChain) {
        let mut table = self.visible_table.lock();
        for link in pgid_chain.links() {
            if core::ptr::eq(Arc::as_ptr(&link.ns), self) {
                table.remove_process_group(link.nr);
            }
        }
    }

    pub fn insert_session_chain(&self, session: &Arc<Session>, sid_chain: &PidChain) {
        let mut table = self.visible_table.lock();
        for link in sid_chain.links() {
            if core::ptr::eq(Arc::as_ptr(&link.ns), self) {
                table.insert_session(link.nr, session);
            }
        }
    }

    pub fn remove_session_chain(&self, sid_chain: &PidChain) {
        let mut table = self.visible_table.lock();
        for link in sid_chain.links() {
            if core::ptr::eq(Arc::as_ptr(&link.ns), self) {
                table.remove_session(link.nr);
            }
        }
    }

    pub(in crate::process) fn make_current_main_thread(ctx: &Context) {
        let pid = ctx.process.pid();
        let old_tid = ctx.posix_thread.tid();

        if old_tid == pid {
            return;
        }

        let mut tasks = ctx.process.tasks().lock();

        assert!(tasks.has_exited_main());
        assert!(tasks.in_execve());
        assert_eq!(tasks.as_slice().len(), 2);
        assert!(core::ptr::eq(ctx.task, tasks.as_slice()[1].as_ref()));

        tasks.swap_main(pid, old_tid);

        let old_tid_chain = ctx.posix_thread.tid_chain().clone();
        let new_tid_chain = ctx.process.pid_chain().clone();
        let thread = ctx.task.as_thread().unwrap().clone();

        Self::remove_thread_across_namespaces(&old_tid_chain);
        ctx.posix_thread.set_main(new_tid_chain);
        Self::insert_thread_across_namespaces(thread);
    }
}

impl PidChain {
    pub fn from_links(numbers: Vec<PidLink>) -> Self {
        Self {
            numbers: numbers.into_boxed_slice(),
        }
    }

    pub fn one(ns: Arc<PidNamespace>, nr: u32) -> Self {
        Self::from_links(vec![PidLink { ns, nr }])
    }

    pub fn links(&self) -> &[PidLink] {
        &self.numbers
    }

    pub fn nr_in(&self, ns: &PidNamespace) -> Option<u32> {
        self.numbers
            .iter()
            .find(|link| core::ptr::eq(Arc::as_ptr(&link.ns), ns))
            .map(|link| link.nr)
    }

    #[expect(dead_code)]
    pub fn contains_ns(&self, ns: &PidNamespace) -> bool {
        self.nr_in(ns).is_some()
    }

    pub fn active_link(&self) -> &PidLink {
        self.numbers.last().unwrap()
    }
}

impl PidLink {
    pub fn ns(&self) -> &Arc<PidNamespace> {
        &self.ns
    }

    pub fn nr(&self) -> u32 {
        self.nr
    }
}

impl PidNsForChildren {
    pub fn target(&self, active: &Arc<PidNamespace>) -> Arc<PidNamespace> {
        match self {
            Self::SameAsActive => active.clone(),
            Self::Target(ns) => ns.clone(),
        }
    }
}

impl NsCommonOps for PidNamespace {
    const TYPE: NsType = NsType::Pid;

    fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        Some(&self.owner_user_ns)
    }

    fn parent(&self) -> Result<&Arc<Self>> {
        self.parent.as_ref().ok_or_else(|| {
            Error::with_message(Errno::EPERM, "the initial pid namespace has no parent")
        })
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
