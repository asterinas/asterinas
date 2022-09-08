//! The std library of kxos
#![no_std]
#![forbid(unsafe_code)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![feature(const_btree_new)]
#![feature(cstr_from_bytes_until_nul)]

use kxos_frame::{debug, info, println};
use process::Process;

use crate::process::current_pid;

extern crate alloc;

mod memory;
mod process;
mod syscall;
mod util;

pub fn init() {
    process::fifo_scheduler::init();
}

pub fn init_process() {
    println!("[kernel] Spawn init process!");

    let process = Process::spawn_kernel_process(|| {
        println!("[kernel] Hello world from kernel!");
        let pid = current_pid();
        debug!("current pid = {}", pid);
    });
    info!(
        "[kxos-std/lib.rs] spawn kernel process, pid = {}",
        process.pid()
    );

    let hello_world_content = read_hello_world_content();
    let process = Process::spawn_user_process(hello_world_content);
    info!(
        "[kxos-std/lib.rs] spwan hello world process, pid = {}",
        process.pid()
    );

    let fork_content = read_fork_content();
    let process = Process::spawn_user_process(fork_content);
    info!(
        "[kxos-std/lib.rs] spawn fork process, pid = {}",
        process.pid()
    );

    loop {}
}

/// first process never return
pub fn run_first_process() -> ! {
    let elf_file_content = read_hello_world_content();
    Process::spawn_kernel_process(init_process);
    unreachable!()
}

pub fn read_hello_world_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/hello_world/hello_world")
}

fn read_fork_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/fork/fork")
}
