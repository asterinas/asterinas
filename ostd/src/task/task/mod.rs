// SPDX-License-Identifier: MPL-2.0

// FIXME: the `intrusive_adapter` macro will generate methods without docs.
// So we temporary allow missing_docs for this module.
#![allow(missing_docs)]

mod priority;

use core::{
    any::Any,
    cell::UnsafeCell,
    sync::atomic::{AtomicU32, Ordering},
};

use intrusive_collections::{intrusive_adapter, LinkedListAtomicLink};
pub use priority::Priority;

use super::{processor::current_task, scheduler};
pub(crate) use crate::arch::task::{context_switch, TaskContext};
use crate::{
    cpu::CpuSet,
    mm::{kspace::KERNEL_PAGE_TABLE, FrameAllocOptions, Paddr, PageFlags, Segment, PAGE_SIZE},
    prelude::*,
    user::UserSpace,
};

/// The kernel stack size of a task, specified in pages.
///
/// By default, we choose a rather large stack size.
/// OSTD users can choose a smaller size by specifying
/// the `OSTD_TASK_STACK_SIZE_IN_PAGES` environment variable
/// at build time.
pub static STACK_SIZE_IN_PAGES: u32 = parse_u32_or_default(
    option_env!("OSTD_TASK_STACK_SIZE_IN_PAGES"),
    DEFAULT_STACK_SIZE_IN_PAGES,
);

/// The default kernel stack size of a task, specified in pages.
pub const DEFAULT_STACK_SIZE_IN_PAGES: u32 = 128;

const fn parse_u32_or_default(size: Option<&str>, default: u32) -> u32 {
    match size {
        Some(value) => parse_u32(value),
        None => default,
    }
}

const fn parse_u32(input: &str) -> u32 {
    let mut output: u32 = 0;
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let digit = (bytes[i] - b'0') as u32;
        output = output * 10 + digit;
        i += 1;
    }
    output
}

/// Trait for manipulating the task context.
pub trait TaskContextApi {
    /// Sets instruction pointer
    fn set_instruction_pointer(&mut self, ip: usize);

    /// Gets instruction pointer
    fn instruction_pointer(&self) -> usize;

    /// Sets stack pointer
    fn set_stack_pointer(&mut self, sp: usize);

    /// Gets stack pointer
    fn stack_pointer(&self) -> usize;
}

#[derive(Debug)]
pub struct KernelStack {
    segment: Segment,
    has_guard_page: bool,
}

impl KernelStack {
    pub fn new() -> Result<Self> {
        Ok(Self {
            segment: FrameAllocOptions::new(STACK_SIZE_IN_PAGES as usize).alloc_contiguous()?,
            has_guard_page: false,
        })
    }

    /// Generates a kernel stack with a guard page.
    /// An additional page is allocated and be regarded as a guard page, which should not be accessed.  
    pub fn new_with_guard_page() -> Result<Self> {
        let stack_segment =
            FrameAllocOptions::new(STACK_SIZE_IN_PAGES as usize + 1).alloc_contiguous()?;
        // FIXME: modifying the the linear mapping is bad.
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let guard_page_vaddr = {
            let guard_page_paddr = stack_segment.start_paddr();
            crate::mm::paddr_to_vaddr(guard_page_paddr)
        };
        // SAFETY: the segment allocated is not used by others so we can protect it.
        unsafe {
            let vaddr_range = guard_page_vaddr..guard_page_vaddr + PAGE_SIZE;
            page_table
                .protect_flush_tlb(&vaddr_range, |p| p.flags -= PageFlags::RW)
                .unwrap();
        }
        Ok(Self {
            segment: stack_segment,
            has_guard_page: true,
        })
    }

    pub fn end_paddr(&self) -> Paddr {
        self.segment.end_paddr()
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        if self.has_guard_page {
            // FIXME: modifying the the linear mapping is bad.
            let page_table = KERNEL_PAGE_TABLE.get().unwrap();
            let guard_page_vaddr = {
                let guard_page_paddr = self.segment.start_paddr();
                crate::mm::paddr_to_vaddr(guard_page_paddr)
            };
            // SAFETY: the segment allocated is not used by others so we can protect it.
            unsafe {
                let vaddr_range = guard_page_vaddr..guard_page_vaddr + PAGE_SIZE;
                page_table
                    .protect_flush_tlb(&vaddr_range, |p| p.flags |= PageFlags::RW)
                    .unwrap();
            }
        }
    }
}

/// An atomic CPUID container.
pub struct AtomicCpuId(AtomicU32);

impl AtomicCpuId {
    /// The null value of CPUID.
    ///
    /// An `AtomicCpuId` with `AtomicCpuId::NONE` as its inner value is empty.
    const NONE: u32 = u32::MAX;

    fn new(cpu_id: u32) -> Self {
        Self(AtomicU32::new(cpu_id))
    }

    /// Sets the inner value of an `AtomicCpuId` if it's empty.
    ///
    /// The return value is a result indicating whether the new value was written
    /// and containing the previous value.
    pub fn set_if_is_none(&self, cpu_id: u32) -> core::result::Result<u32, u32> {
        self.0
            .compare_exchange(Self::NONE, cpu_id, Ordering::Relaxed, Ordering::Relaxed)
    }

    /// Sets the inner value of an `AtomicCpuId` to `AtomicCpuId::NONE`, i.e. makes
    /// an `AtomicCpuId` empty.
    pub fn set_to_none(&self) {
        self.0.store(Self::NONE, Ordering::Relaxed);
    }
}

impl Default for AtomicCpuId {
    fn default() -> Self {
        Self::new(Self::NONE)
    }
}

/// A task that executes a function to the end.
///
/// Each task is associated with per-task data and an optional user space.
/// If having a user space, the task can switch to the user space to
/// execute user code. Multiple tasks can share a single user space.
pub struct Task {
    func: Box<dyn Fn() + Send + Sync>,
    data: Box<dyn Any + Send + Sync>,
    user_space: Option<Arc<UserSpace>>,
    ctx: UnsafeCell<TaskContext>,
    /// kernel stack, note that the top is SyscallFrame/TrapFrame
    kstack: KernelStack,
    link: LinkedListAtomicLink,
    cpu: AtomicCpuId,
    priority: Priority,
    // TODO: add multiprocessor support
    #[allow(dead_code)]
    cpu_affinity: CpuSet,
}

// TaskAdapter struct is implemented for building relationships between doubly linked list and Task struct
intrusive_adapter!(pub TaskAdapter = Arc<Task>: Task { link: LinkedListAtomicLink });

// SAFETY: `UnsafeCell<TaskContext>` is not `Sync`. However, we only use it in `schedule()` where
// we have exclusive access to the field.
unsafe impl Sync for Task {}

impl Task {
    /// Gets the current task.
    ///
    /// It returns `None` if the function is called in the bootstrap context.
    pub fn current() -> Option<Arc<Task>> {
        current_task()
    }

    pub(super) fn ctx(&self) -> &UnsafeCell<TaskContext> {
        &self.ctx
    }

    /// Yields execution so that another task may be scheduled.
    ///
    /// Note that this method cannot be simply named "yield" as the name is
    /// a Rust keyword.
    pub fn yield_now() {
        scheduler::yield_now()
    }

    /// Runs the task.
    ///
    /// BUG: This method highly depends on the current scheduling policy.
    pub fn run(self: &Arc<Self>) {
        scheduler::run_new_task(self.clone());
    }

    /// Returns the task data.
    pub fn data(&self) -> &Box<dyn Any + Send + Sync> {
        &self.data
    }

    /// Returns the user space of this task, if it has.
    pub fn user_space(&self) -> Option<&Arc<UserSpace>> {
        if self.user_space.is_some() {
            Some(self.user_space.as_ref().unwrap())
        } else {
            None
        }
    }

    // Returns the cpu of this task.
    pub fn cpu(&self) -> &AtomicCpuId {
        &self.cpu
    }

    /// Returns the priority.
    pub fn priority(&self) -> Priority {
        self.priority
    }

    /// Exits the current task.
    ///
    /// The task `self` must be the task that is currently running.
    ///
    /// **NOTE:** If there is anything left on the stack, it will be forgotten. This behavior may
    /// lead to resource leakage.
    fn exit(self: Arc<Self>) -> ! {
        // `current_task()` still holds a strong reference, so nothing is destroyed at this point,
        // neither is the kernel stack.
        drop(self);
        scheduler::exit_current();
        unreachable!()
    }

    /// Checks if the task has a real-time priority.
    pub fn is_real_time(&self) -> bool {
        self.priority.is_real_time()
    }
}

/// Options to create or spawn a new task.
pub struct TaskOptions {
    func: Option<Box<dyn Fn() + Send + Sync>>,
    data: Option<Box<dyn Any + Send + Sync>>,
    user_space: Option<Arc<UserSpace>>,
    priority: Priority,
    cpu_affinity: CpuSet,
}

impl TaskOptions {
    /// Creates a set of options for a task.
    pub fn new<F>(func: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        Self {
            func: Some(Box::new(func)),
            data: None,
            user_space: None,
            priority: Priority::normal(),
            cpu_affinity: CpuSet::new_full(),
        }
    }

    /// Sets the function that represents the entry point of the task.
    pub fn func<F>(mut self, func: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.func = Some(Box::new(func));
        self
    }

    /// Sets the data associated with the task.
    pub fn data<T>(mut self, data: T) -> Self
    where
        T: Any + Send + Sync,
    {
        self.data = Some(Box::new(data));
        self
    }

    /// Sets the user space associated with the task.
    pub fn user_space(mut self, user_space: Option<Arc<UserSpace>>) -> Self {
        self.user_space = user_space;
        self
    }

    /// Sets the priority of the task.
    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Sets the CPU affinity mask for the task.
    ///
    /// The `cpu_affinity` parameter represents
    /// the desired set of CPUs to run the task on.
    pub fn cpu_affinity(mut self, cpu_affinity: CpuSet) -> Self {
        self.cpu_affinity = cpu_affinity;
        self
    }

    /// Builds a new task without running it immediately.
    pub fn build(self) -> Result<Arc<Task>> {
        /// all task will entering this function
        /// this function is mean to executing the task_fn in Task
        extern "C" fn kernel_task_entry() {
            let current_task = current_task()
                .expect("no current task, it should have current task in kernel task entry");
            current_task.func.call(());
            current_task.exit();
        }

        let mut new_task = Task {
            func: self.func.unwrap(),
            data: self.data.unwrap(),
            user_space: self.user_space,
            ctx: UnsafeCell::new(TaskContext::default()),
            kstack: KernelStack::new_with_guard_page()?,
            cpu: AtomicCpuId::default(),
            link: LinkedListAtomicLink::new(),
            priority: self.priority,
            cpu_affinity: self.cpu_affinity,
        };

        let ctx = new_task.ctx.get_mut();
        ctx.set_instruction_pointer(kernel_task_entry as usize);
        // We should reserve space for the return address in the stack, otherwise
        // we will write across the page boundary due to the implementation of
        // the context switch.
        //
        // According to the System V AMD64 ABI, the stack pointer should be aligned
        // to at least 16 bytes. And a larger alignment is needed if larger arguments
        // are passed to the function. The `kernel_task_entry` function does not
        // have any arguments, so we only need to align the stack pointer to 16 bytes.
        ctx.set_stack_pointer(crate::mm::paddr_to_vaddr(new_task.kstack.end_paddr() - 16));

        Ok(Arc::new(new_task))
    }

    /// Builds a new task and run it immediately.
    pub fn spawn(self) -> Result<Arc<Task>> {
        let task = self.build()?;
        task.run();
        Ok(task)
    }
}

#[cfg(ktest)]
mod test {
    use crate::prelude::*;

    #[ktest]
    fn create_task() {
        #[allow(clippy::eq_op)]
        let task = || {
            assert_eq!(1, 1);
        };
        let task_option = crate::task::TaskOptions::new(task)
            .data(())
            .build()
            .unwrap();
        task_option.run();
    }

    #[ktest]
    fn spawn_task() {
        #[allow(clippy::eq_op)]
        let task = || {
            assert_eq!(1, 1);
        };
        let _ = crate::task::TaskOptions::new(task).data(()).spawn();
    }
}
