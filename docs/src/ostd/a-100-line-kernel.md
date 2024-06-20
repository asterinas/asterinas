# Example: Writing a Kernel in About 100 Lines of Safe Rust

To give you a sense of
how Asterinas OSTD enables writing kernels in safe Rust,
we will show a new kernel in about 100 lines of safe Rust.

Our new kernel will be able to run the following Hello World program.

```s
.global _start                      # entry point
.section .text                      # code section
_start:
    mov     $1, %rax                # syscall number of write
    mov     $1, %rdi                # stdout
    mov     $message, %rsi          # address of message         
    mov     $message_end, %rdx
    sub     %rsi, %rdx              # calculate message len
    syscall
    mov     $60, %rax               # syscall number of exit, move it to rax
    mov     $0, %rdi                # exit code, move it to rdi
    syscall  

.section .rodata                    # read only data section
message:
    .ascii  "Hello, world\n"
message_end:
```

The assembly program above can be compiled with the following command.

```bash
gcc -static -nostdlib hello.S -o hello
```

The user program above requires our kernel to support three main features:
1. Loading a program as a process image in user space;
3. Handling the write system call;
4. Handling the exit system call.

A sample implementation of the kernel in safe Rust is given below.
Comments are added
to highlight how the APIs of Asterinas OSTD enable safe kernel development.

```rust
#![no_std]

extern crate alloc;

use align_ext::AlignExt;
use core::str;

use alloc::sync::Arc;
use alloc::vec;

use ostd::cpu::UserContext;
use ostd::prelude::*;
use ostd::task::{Task, TaskOptions};
use ostd::user::{ReturnReason, UserMode, UserSpace};
use ostd::mm::{PageFlags, PAGE_SIZE, Vaddr, FrameAllocOptions, VmIo, VmMapOptions, VmSpace};

/// The kernel's boot and initialization process is managed by Asterinas OSTD.
/// After the process is done, the kernel's execution environment
/// (e.g., stack, heap, tasks) will be ready for use and the entry function
/// labeled as `#[ostd::main]` will be called.
#[ostd::main]
pub fn main() {
    let program_binary = include_bytes!("../hello_world");
    let user_space = create_user_space(program_binary);
    let user_task = create_user_task(Arc::new(user_space));
    user_task.run();
}

fn create_user_space(program: &[u8]) -> UserSpace {
    let user_pages = {
        let nframes = program.len().align_up(PAGE_SIZE) / PAGE_SIZE;
        let vm_frames = FrameAllocOptions::new(nframes).alloc().unwrap();
        // Phyiscal memory pages can be only accessed
        // via the Frame abstraction.
        vm_frames.write_bytes(0, program).unwrap();
        vm_frames
    };
    let user_address_space = {
        const MAP_ADDR: Vaddr = 0x0040_0000; // The map addr for statically-linked executable

        // The page table of the user space can be
        // created and manipulated safely through
        // the VmSpace abstraction.
        let vm_space = VmSpace::new();
        let mut options = VmMapOptions::new();
        options.addr(Some(MAP_ADDR)).flags(PageFlags::RWX);
        vm_space.map(user_pages, &options).unwrap();
        vm_space
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
        let current = Task::current();
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

    // Kernel tasks are managed by OSTD,
    // while scheduling algorithms for them can be
    // determined by the users of OSTD.
    TaskOptions::new(user_task)
        .user_space(Some(user_space))
        .data(0)
        .build()
        .unwrap()
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
                user_space
                    .vm_space()
                    .read_bytes(buf_addr, &mut buf)
                    .unwrap();
                buf
            };
            // Use the console for output safely.
            println!("{}", str::from_utf8(&buf).unwrap());
            // Manipulate the user-space CPU registers safely.
            user_context.set_rax(buf_len);
        }
        SYS_EXIT => Task::current().exit(),
        _ => unimplemented!(),
    }
}
```

