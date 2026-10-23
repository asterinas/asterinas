// SPDX-License-Identifier: MPL-2.0

use ostd::{arch::cpu::context::UserContext, task::Task};

use super::{
    Credentials, Process, ProcessVm, UserNamespace,
    posix_thread::{PosixThreadBuilder, allocate_posix_tid, derive_thread_name},
    signal::sig_disposition::SigDispositions,
};
use crate::{fs::pseudofs::SockFs, prelude::*, sched::Nice, vm::vmar::VmarHandle};

// Use real POSIX tasks so that WaitQueue exercises signal cancellation,
// rather than the non-interruptible fallback for kernel threads.
pub(super) fn new_process() -> (Arc<Process>, VmarHandle) {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();
    let vmar = VmarHandle::new(ProcessVm::new(SockFs::new_path()));
    let process = Process::new(
        allocate_posix_tid(),
        vmar.clone_arc(),
        Default::default(),
        Nice::default(),
        0,
        Arc::new(Mutex::new(SigDispositions::new())),
        UserNamespace::get_init_singleton().clone(),
    );
    (process, vmar)
}

pub(super) fn new_task(
    process: &Arc<Process>,
    vmar: &VmarHandle,
    entry: impl FnOnce() + Send + 'static,
) -> Arc<Task> {
    PosixThreadBuilder::new(
        allocate_posix_tid(),
        derive_thread_name("cred-guard-test"),
        Box::new(UserContext::default()),
        Credentials::new_root(),
        vmar.clone_handle(),
    )
    .process(Arc::downgrade(process))
    .build_for_test(entry)
}

mod exit_arbitration {
    use core::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };

    use ostd::prelude::ktest;

    use super::*;
    use crate::{
        process::{
            TermStatus,
            posix_thread::{self, AsPosixThread},
            signal::{HandlePendingSignal, constants::SIGSYS},
        },
        thread::AsThread,
    };

    fn exit_sibling(process: &Arc<Process>, vmar: &VmarHandle, before_run_fn: impl FnOnce()) {
        let finished = Arc::new(AtomicBool::new(false));
        let done = Arc::new(ostd::sync::WaitQueue::new());
        let task = new_task(process, vmar, {
            let process = process.clone();
            let finished = finished.clone();
            let done = done.clone();
            move || {
                let current = Task::current().unwrap();
                let thread = current.as_thread().unwrap();
                let ctx = Context {
                    process,
                    thread_local: current.as_thread_local().unwrap(),
                    posix_thread: current.as_posix_thread().unwrap(),
                    thread,
                    task: &current,
                };
                posix_thread::do_seccomp_exit_group(&ctx, &mut UserContext::default());
                finished.store(true, Ordering::Release);
                done.wake_all();
            }
        });
        process.tasks().lock().insert(task.clone()).unwrap();
        before_run_fn();
        task.as_thread().unwrap().run();
        // do_exit marks the thread exited before finishing resource cleanup;
        // Thread::join alone would not wait for the assertions' prerequisites.
        done.wait_until_or_timeout(
            || finished.load(Ordering::Acquire).then_some(()),
            &Duration::from_secs(5),
        )
        .unwrap();
    }

    #[ktest]
    fn seccomp_group_exit_preserves_an_exec_already_in_progress() {
        let (process, vmar) = new_process();
        // Keep a main task alive so this test exercises sibling exit, without
        // invoking process reparenting or requiring a global init process.
        let main = new_task(&process, &vmar, || {});
        process.tasks().lock().insert(main.clone()).unwrap();
        exit_sibling(&process, &vmar, || process.tasks().lock().start_execve());
        let tasks = process.tasks().lock();
        assert!(tasks.in_execve());
        assert!(!tasks.has_exited_group());
        assert_eq!(tasks.as_slice().len(), 1);
        assert!(!main.as_posix_thread().unwrap().has_pending_sigkill());
        assert_eq!(process.status().exit_code(), 0);
    }

    #[ktest]
    fn seccomp_group_exit_publishes_fatal_status_and_kills_siblings() {
        let (process, vmar) = new_process();
        let main = new_task(&process, &vmar, || {});
        process.tasks().lock().insert(main.clone()).unwrap();
        exit_sibling(&process, &vmar, || {});
        let mut tasks = process.tasks().lock();
        assert!(tasks.has_exited_group());
        assert!(!tasks.in_execve());
        assert!(main.as_posix_thread().unwrap().has_pending_sigkill());
        assert_eq!(
            process.status().exit_code(),
            TermStatus::Killed(SIGSYS).as_u32()
        );
        assert!(tasks.insert(main.clone()).is_err());
    }
}
