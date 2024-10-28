// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use align_ext::AlignExt;
use core::str;

use alloc::sync::Arc;
use alloc::vec;

use ostd::arch::qemu::{exit_qemu, QemuExitCode};
use ostd::cpu::UserContext;
use ostd::mm::{
    CachePolicy, FallibleVmRead, FallibleVmWrite, FrameAllocOptions, PageFlags, PageProperty,
    Vaddr, VmIo, VmSpace, VmWriter, PAGE_SIZE,
};
use ostd::prelude::*;
use ostd::task::{Task, TaskOptions};
use ostd::user::{ReturnReason, UserMode, UserSpace};

/// The kernel's boot and initialization process is managed by OSTD.
/// After the process is done, the kernel's execution environment
/// (e.g., stack, heap, tasks) will be ready for use and the entry function
/// labeled as `#[ostd::main]` will be called.
#[ostd::main]
pub fn main() {
    let program_binary = include_bytes!("../hello");
    let user_space = create_user_space(program_binary);
    let user_task = create_user_task(Arc::new(user_space));
    user_task.run();
}

fn create_user_space(program: &[u8]) -> UserSpace {
    let nbytes = program.len().align_up(PAGE_SIZE);
    let user_pages = {
        let segment = FrameAllocOptions::new(nbytes / PAGE_SIZE)
            .alloc_contiguous()
            .unwrap();
        // Physical memory pages can be only accessed
        // via the `Frame` or `Segment` abstraction.
        segment.write_bytes(0, program).unwrap();
        segment
    };
    let user_address_space = {
        const MAP_ADDR: Vaddr = 0x0040_0000; // The map addr for statically-linked executable

        // The page table of the user space can be
        // created and manipulated safely through
        // the `VmSpace` abstraction.
        let vm_space = VmSpace::new();
        vm_space
            .cursor_mut_with(&(MAP_ADDR..MAP_ADDR + nbytes), |cursor| {
                let map_prop = PageProperty::new(PageFlags::RWX, CachePolicy::Writeback);
                for frame in user_pages {
                    cursor.map(frame, map_prop);
                }
            })
            .unwrap();
        Arc::new(vm_space)
    };
    let user_cpu_state = {
        const ENTRY_POINT: Vaddr = 0x0040_1000; // The entry point for statically-linked executable

        // The user-space CPU states can be initialized
        // to arbitrary values via the UserContext
        // abstraction.
        let mut user_cpu_state = UserContext::default();
        user_cpu_state.set_rip(ENTRY_POINT);
        user_cpu_state
    };
    UserSpace::new(user_address_space, user_cpu_state)
}

fn create_user_task(user_space: Arc<UserSpace>) -> Arc<Task> {
    fn user_task() {
        let current = Task::current().unwrap();
        // Switching between user-kernel space is
        // performed via the UserMode abstraction.
        let mut user_mode = {
            let user_space = current.user_space().unwrap();
            UserMode::new(user_space)
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
                handle_syscall(user_context, current.user_space().unwrap());
            }
        }
    }

    // Kernel tasks are managed by the Framework,
    // while scheduling algorithms for them can be
    // determined by the users of the Framework.
    Arc::new(
        TaskOptions::new(user_task)
            .user_space(Some(user_space))
            .data(0)
            .build()
            .unwrap(),
    )
}

fn handle_syscall(user_context: &mut UserContext, user_space: &UserSpace) {
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
                let current_vm_space = user_space.vm_space();
                let mut reader = current_vm_space.reader(buf_addr, buf_len).unwrap();
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
