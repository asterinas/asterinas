// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::CpuSet,
    task::{Task, TaskOptions},
};

use super::{AsThread, Thread, oops};
use crate::{
    prelude::*,
    sched::{Nice, SchedPolicy},
    vm::vmar::Vmar,
};

/// The inner data of a kernel thread.
struct KernelThread {
    user_vmar: Option<Arc<Vmar>>,
}

/// Options to create or spawn a new kernel thread.
pub struct ThreadOptions {
    func: Option<Box<dyn FnOnce() + Send>>,
    cpu_affinity: CpuSet,
    sched_policy: SchedPolicy,
    user_vmar: Option<Arc<Vmar>>,
}

impl ThreadOptions {
    /// Creates the thread options with the thread function.
    pub fn new<F>(func: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let cpu_affinity = CpuSet::new_full();
        let sched_policy = SchedPolicy::Fair(Nice::default());
        Self {
            func: Some(Box::new(func)),
            cpu_affinity,
            sched_policy,
            user_vmar: None,
        }
    }

    /// Sets the CPU affinity of the new thread.
    pub fn cpu_affinity(mut self, cpu_affinity: CpuSet) -> Self {
        self.cpu_affinity = cpu_affinity;
        self
    }

    /// Sets the scheduling policy.
    pub fn sched_policy(mut self, sched_policy: SchedPolicy) -> Self {
        self.sched_policy = sched_policy;
        self
    }

    /// Associates a userspace VMAR with the kernel thread.
    ///
    /// The VMAR is activated whenever the thread is scheduled, allowing fallible userspace memory
    /// operations to access it directly and handle page faults against it. The association cannot
    /// be changed after the thread is built.
    ///
    /// Access the associated address space through `VmSpace::reader` and `VmSpace::writer`. Such
    /// accesses may handle page faults and block, so they must not be performed in atomic mode or
    /// while holding a spin lock.
    ///
    /// The `Arc` keeps the VMAR object alive, but does not prevent its mappings from being cleared
    /// after the last [`VmarHandle`](crate::vm::vmar::VmarHandle) is dropped. A caller that needs
    /// the mappings to outlive their userspace owner must hold a separate owner-memory lease.
    #[cfg_attr(
        not(ktest),
        expect(dead_code, reason = "used by the follow-up in-kernel vhost worker")
    )]
    pub(crate) fn user_vmar(mut self, user_vmar: Arc<Vmar>) -> Self {
        self.user_vmar = Some(user_vmar);
        self
    }
}

impl ThreadOptions {
    /// Builds a new kernel thread without running it immediately.
    pub fn build(mut self) -> Arc<Task> {
        let task_fn = self.func.take().unwrap();
        let thread_fn = move || {
            let _ = oops::catch_panics_as_oops(task_fn);
            // Ensure that the thread exits.
            current_thread!().exit();
        };

        Arc::new_cyclic(|weak_task| {
            let thread = {
                let kernel_thread = KernelThread {
                    user_vmar: self.user_vmar,
                };
                let cpu_affinity = self.cpu_affinity;
                let sched_policy = self.sched_policy;
                Arc::new(Thread::new(
                    weak_task.clone(),
                    kernel_thread,
                    cpu_affinity,
                    sched_policy,
                ))
            };

            TaskOptions::new(thread_fn).data(thread).build().unwrap()
        })
    }

    /// Builds a new kernel thread and runs it immediately.
    #[track_caller]
    pub fn spawn(self) -> Arc<Thread> {
        let task = self.build();
        let thread = task.as_thread().unwrap().clone();
        thread.run();
        thread
    }
}

impl Thread {
    /// Returns the userspace VMAR associated with this kernel thread.
    pub(crate) fn user_vmar(&self) -> Option<&Arc<Vmar>> {
        self.data()
            .downcast_ref::<KernelThread>()?
            .user_vmar
            .as_ref()
    }
}

#[cfg(ktest)]
mod tests {
    use core::sync::atomic::{AtomicBool, Ordering};

    use ostd::{cpu::CpuId, prelude::ktest};

    use super::*;
    use crate::{
        process::ProcessVm,
        vm::{
            page_cache::VmoOptions,
            perms::VmPerms,
            vmar::{VmarHandle, VmarMapOffset},
        },
    };

    #[ktest]
    fn associated_vmar_is_reactivated_after_context_switch() {
        super::super::init();

        let vmar = VmarHandle::new(ProcessVm::new_for_test());
        let map_addr = PAGE_SIZE * 16;
        let map_size = PAGE_SIZE * 4;
        let vmo = VmoOptions::new(map_size).alloc().unwrap();
        assert_eq!(
            vmar.new_map(map_size, VmPerms::READ | VmPerms::WRITE)
                .unwrap()
                .offset(VmarMapOffset::FixedNoReplace(map_addr))
                .vmo(vmo)
                .build()
                .unwrap(),
            map_addr
        );

        let access_addr = map_addr + 17;
        let access_len = PAGE_SIZE * 3 + 37;
        let mut expected = vec![0; access_len];
        for (index, byte) in expected.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }

        let source = expected.clone();
        let result = Arc::new(Mutex::new(None));
        let worker_result = result.clone();
        let worker_vmar = vmar.clone_arc();
        let associated_vmar = worker_vmar.clone();
        let owner_has_written = Arc::new(AtomicBool::new(false));
        let switcher_has_run = Arc::new(AtomicBool::new(false));
        let test_cpu = CpuId::current_racy();

        let switcher_vmar = VmarHandle::new(ProcessVm::new_for_test());
        let switcher_user_vmar = switcher_vmar.clone_arc();
        let associated_switcher_vmar = switcher_user_vmar.clone();
        let switcher_owner_has_written = owner_has_written.clone();
        let switcher_done = switcher_has_run.clone();
        let switcher = ThreadOptions::new(move || {
            while !switcher_owner_has_written.load(Ordering::Acquire) {
                Thread::yield_now();
            }

            let current = Thread::current().unwrap();
            assert!(
                current
                    .user_vmar()
                    .is_some_and(|vmar| Arc::ptr_eq(vmar, &switcher_user_vmar))
            );
            assert!(switcher_user_vmar.vm_space().reader(map_addr, 0).is_ok());
            switcher_done.store(true, Ordering::Release);
        })
        .cpu_affinity(test_cpu.into())
        .user_vmar(associated_switcher_vmar)
        .spawn();

        let worker_owner_has_written = owner_has_written.clone();
        let worker_switcher_has_run = switcher_has_run.clone();

        let worker = ThreadOptions::new(move || {
            let current = Thread::current().unwrap();
            assert!(
                current
                    .user_vmar()
                    .is_some_and(|vmar| Arc::ptr_eq(vmar, &worker_vmar))
            );

            let mut user_writer = worker_vmar
                .vm_space()
                .writer(access_addr, access_len)
                .unwrap();
            let mut source_reader = VmReader::from(source.as_slice()).to_fallible();
            assert_eq!(
                user_writer.write_fallible(&mut source_reader).unwrap(),
                access_len
            );

            worker_owner_has_written.store(true, Ordering::Release);
            while !worker_switcher_has_run.load(Ordering::Acquire) {
                Thread::yield_now();
            }

            let mut user_reader = worker_vmar
                .vm_space()
                .reader(access_addr, access_len)
                .unwrap();
            let mut output = vec![0; access_len];
            let mut output_writer = VmWriter::from(output.as_mut_slice()).to_fallible();
            assert_eq!(
                user_reader.read_fallible(&mut output_writer).unwrap(),
                access_len
            );
            *worker_result.lock() = Some(output);
        })
        .cpu_affinity(test_cpu.into())
        .user_vmar(associated_vmar)
        .spawn();

        worker.join();
        switcher.join();
        assert_eq!(result.lock().take().unwrap(), expected);
    }
}
