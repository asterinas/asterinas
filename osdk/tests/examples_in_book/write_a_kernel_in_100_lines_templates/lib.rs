// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use align_ext::AlignExt;
use core::str;

use alloc::sync::Arc;
use alloc::vec;

use ostd::arch::qemu::{exit_qemu, QemuExitCode};
use ostd::cpu::context::UserContext;
use ostd::mm::{
    CachePolicy, FallibleVmRead, FrameAllocOptions, PageFlags, PageProperty, Vaddr, VmIo, VmSpace,
    VmWriter, PAGE_SIZE,
};
use ostd::prelude::*;
use ostd::task::{disable_preempt, Task, TaskOptions};
use ostd::user::{ReturnReason, UserMode};

/// The kernel's boot and initialization process is managed by OSTD.
/// After the process is done, the kernel's execution environment
/// (e.g., stack, heap, tasks) will be ready for use and the entry function
/// labeled as `#[ostd::main]` will be called.
#[ostd::main]
pub fn main() {
    let program_binary = include_bytes!("../hello");
    let vm_space = Arc::new(create_vm_space(program_binary));
    vm_space.activate();
    let user_task = create_user_task(vm_space);
    user_task.run();
}

fn create_vm_space(program: &[u8]) -> VmSpace {
    let nbytes = program.len().align_up(PAGE_SIZE);
    let user_pages = {
        let segment = FrameAllocOptions::new()
            .alloc_segment(nbytes / PAGE_SIZE)
            .unwrap();
        // Physical memory pages can be only accessed
        // via the `UFrame` or `USegment` abstraction.
        segment.write_bytes(0, program).unwrap();
        segment
    };

    // The page table of the user space can be
    // created and manipulated safely through
    // the `VmSpace` abstraction.
    let vm_space = VmSpace::new();
    const MAP_ADDR: Vaddr = 0x0040_0000; // The map addr for statically-linked executable
    let preempt_guard = disable_preempt();
    let mut cursor = vm_space
        .cursor_mut(&preempt_guard, &(MAP_ADDR..MAP_ADDR + nbytes))
        .unwrap();
    let map_prop = PageProperty::new_user(PageFlags::RWX, CachePolicy::Writeback);
    for frame in user_pages {
        cursor.map(frame.into(), map_prop);
    }
    drop(cursor);
    vm_space
}

fn create_user_task(vm_space: Arc<VmSpace>) -> Arc<Task> {
    fn user_task() {
        let current = Task::current().unwrap();
        // Switching between user-kernel space is
        // performed via the UserMode abstraction.
        let mut user_mode = {
            let user_ctx = create_user_context();
            UserMode::new(user_ctx)
        };

        loop {
            // The execute method returns when system
            // calls or CPU exceptions occur or some
            // events specified by the kernel occur.
            let return_reason = user_mode.execute(|| false);

            // The CPU registers of the user space
            // can be accessed and manipulated via
            // the `UserContext` abstraction.
            let user_context = user_mode.context_mut();
            if ReturnReason::UserSyscall == return_reason {
                let vm_space = current.data().downcast_ref::<Arc<VmSpace>>().unwrap();
                handle_syscall(user_context, &vm_space);
            }
        }
    }

    // Kernel tasks are managed by the Framework,
    // while scheduling algorithms for them can be
    // determined by the users of the Framework.
    Arc::new(TaskOptions::new(user_task).data(vm_space).build().unwrap())
}

fn create_user_context() -> UserContext {
    // The user-space CPU states can be initialized
    // to arbitrary values via the `UserContext`
    // abstraction.
    let mut user_ctx = UserContext::default();
    const ENTRY_POINT: Vaddr = 0x0040_1000; // The entry point for statically-linked executable
    user_ctx.set_rip(ENTRY_POINT);
    user_ctx
}

fn handle_syscall(user_context: &mut UserContext, vm_space: &VmSpace) {
    const SYS_WRITE: usize = 1;
    const SYS_EXIT: usize = 60;

    match user_context.rax() {
        SYS_WRITE => {
            // Access the user-space CPU registers safely.
            let (_, buf_addr, buf_len) =
                (user_context.rdi(), user_context.rsi(), user_context.rdx());
            let buf = {
                let mut buf = vec![0u8; buf_len];
                // Copy data from the user space without
                // unsafe pointer dereferencing.
                let mut reader = vm_space.reader(buf_addr, buf_len).unwrap();
                reader
                    .read_fallible(&mut VmWriter::from(&mut buf as &mut [u8]))
                    .unwrap();
                buf
            };
            // Use the console for output safely.
            println!("{}", str::from_utf8(&buf).unwrap());
            // Manipulate the user-space CPU registers safely.
            user_context.set_rax(buf_len);
        }
        SYS_EXIT => exit_qemu(QemuExitCode::Success),
        _ => unimplemented!(),
    }
}
