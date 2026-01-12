// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, AtomicU64};

use ostd::{
    arch::cpu::context::{FpuContext, UserContext},
    cpu::CpuSet,
    sync::RwArc,
    task::Task,
};

use super::{PosixThread, ThreadLocal, thread_table};
use crate::{
    fs::{file_table::FileTable, thread_info::ThreadFsInfo},
    prelude::*,
    process::{
        Credentials, NsProxy, Process, UserNamespace,
        posix_thread::name::ThreadName,
        signal::{sig_mask::AtomicSigMask, sig_queues::SigQueues},
    },
    sched::{Nice, SchedPolicy},
    thread::{Thread, Tid, task},
    time::{TimerManager, clocks::ProfClock},
};

/// The builder to build a posix thread
pub struct PosixThreadBuilder {
    // The essential part
    tid: Tid,
    thread_name: ThreadName,
    user_ctx: Box<UserContext>,
    process: Weak<Process>,
    credentials: Credentials,

    // Optional part
    set_child_tid: Vaddr,
    clear_child_tid: Vaddr,
    file_table: Option<RwArc<FileTable>>,
    fs: Option<Arc<ThreadFsInfo>>,
    sig_mask: AtomicSigMask,
    sig_queues: SigQueues,
    sched_policy: SchedPolicy,
    fpu_context: FpuContext,
    user_ns: Option<Arc<UserNamespace>>,
    ns_proxy: Option<Arc<NsProxy>>,
    is_init_process: bool,
    default_timer_slack_ns: u64,
}

impl PosixThreadBuilder {
    pub fn new(
        tid: Tid,
        thread_name: ThreadName,
        user_ctx: Box<UserContext>,
        credentials: Credentials,
    ) -> Self {
        Self {
            tid,
            thread_name,
            user_ctx,
            process: Weak::new(),
            credentials,
            set_child_tid: 0,
            clear_child_tid: 0,
            file_table: None,
            fs: None,
            sig_mask: AtomicSigMask::new_empty(),
            sig_queues: SigQueues::new(),
            sched_policy: SchedPolicy::Fair(Nice::default()),
            fpu_context: FpuContext::new(),
            is_init_process: false,
            user_ns: None,
            ns_proxy: None,
            default_timer_slack_ns: 50_000, // 50 usec default slack
        }
    }

    pub fn process(mut self, process: Weak<Process>) -> Self {
        self.process = process;
        self
    }

    pub fn set_child_tid(mut self, set_child_tid: Vaddr) -> Self {
        self.set_child_tid = set_child_tid;
        self
    }

    pub fn clear_child_tid(mut self, clear_child_tid: Vaddr) -> Self {
        self.clear_child_tid = clear_child_tid;
        self
    }

    pub fn file_table(mut self, file_table: RwArc<FileTable>) -> Self {
        self.file_table = Some(file_table);
        self
    }

    pub fn fs(mut self, fs: Arc<ThreadFsInfo>) -> Self {
        self.fs = Some(fs);
        self
    }

    pub fn sig_mask(mut self, sig_mask: AtomicSigMask) -> Self {
        self.sig_mask = sig_mask;
        self
    }

    pub fn fpu_context(mut self, fpu_context: FpuContext) -> Self {
        self.fpu_context = fpu_context;
        self
    }

    pub fn user_ns(mut self, user_ns: Arc<UserNamespace>) -> Self {
        self.user_ns = Some(user_ns);
        self
    }

    pub fn ns_proxy(mut self, ns_proxy: Arc<NsProxy>) -> Self {
        self.ns_proxy = Some(ns_proxy);
        self
    }

    pub fn default_timer_slack_ns(mut self, slack_ns: u64) -> Self {
        self.default_timer_slack_ns = slack_ns;
        self
    }

    #[expect(clippy::wrong_self_convention)]
    pub(in crate::process) fn is_init_process(mut self) -> Self {
        self.is_init_process = true;
        self
    }

    pub fn build(self) -> Arc<Task> {
        let Self {
            tid,
            user_ctx,
            process,
            credentials,
            thread_name,
            set_child_tid,
            clear_child_tid,
            file_table,
            fs,
            sig_mask,
            sig_queues,
            sched_policy,
            fpu_context,
            user_ns,
            ns_proxy,
            is_init_process,
            default_timer_slack_ns,
        } = self;

        let file_table = file_table.unwrap_or_else(|| RwArc::new(FileTable::new()));

        assert_eq!(user_ns.is_none(), ns_proxy.is_none());
        let user_ns = user_ns.unwrap_or_else(|| UserNamespace::get_init_singleton().clone());
        let ns_proxy = ns_proxy.unwrap_or_else(|| NsProxy::get_init_singleton().clone());

        let fs = fs
            .unwrap_or_else(|| Arc::new(ThreadFsInfo::new(ns_proxy.mnt_ns().new_path_resolver())));

        let vmar = process.upgrade().unwrap().lock_vmar().dup_vmar().unwrap();

        Arc::new_cyclic(|weak_task| {
            let posix_thread = {
                let prof_clock = ProfClock::new();
                let virtual_timer_manager = TimerManager::new(prof_clock.user_clock().clone());
                let prof_timer_manager = TimerManager::new(prof_clock.clone());

                PosixThread {
                    process,
                    task: weak_task.clone(),
                    tid: AtomicU32::new(tid),
                    name: Mutex::new(thread_name),
                    credentials,
                    fs: RwMutex::new(fs.clone()),
                    file_table: Mutex::new(Some(file_table.clone_ro())),
                    sig_mask,
                    sig_queues,
                    signalled_waker: SpinLock::new(None),
                    prof_clock,
                    virtual_timer_manager,
                    prof_timer_manager,
                    io_priority: AtomicU32::new(0),
                    ns_proxy: Mutex::new(Some(ns_proxy.clone())),
                    timer_slack_ns: AtomicU64::new(default_timer_slack_ns),
                    default_timer_slack_ns: AtomicU64::new(default_timer_slack_ns),
                }
            };

            let cpu_affinity = CpuSet::new_full();
            let thread = Arc::new(Thread::new(
                weak_task.clone(),
                posix_thread,
                cpu_affinity,
                sched_policy,
            ));

            let thread_local = ThreadLocal::new(
                set_child_tid,
                clear_child_tid,
                vmar,
                file_table,
                fs,
                fpu_context,
                user_ns,
                ns_proxy,
            );

            thread_table::add_thread(tid, thread.clone());
            task::create_new_user_task(user_ctx, thread, thread_local, is_init_process)
        })
    }
}
