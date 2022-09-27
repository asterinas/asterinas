//! The std library of kxos
#![no_std]
#![forbid(unsafe_code)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![feature(const_btree_new)]
#![feature(cstr_from_bytes_until_nul)]
#![feature(half_open_range_patterns)]
#![feature(exclusive_range_pattern)]
#![feature(btree_drain_filter)]

use alloc::ffi::CString;
use kxos_frame::{debug, info, println};
use process::Process;

extern crate alloc;

pub mod driver;
mod memory;
mod process;
pub mod syscall;
mod util;

pub fn init() {
    driver::init();
    process::fifo_scheduler::init();
}

pub fn init_process() {
    println!("[kernel] Spawn init process!");
    driver::pci::virtio::block::block_device_test();
    let process = Process::spawn_kernel_process(|| {
        println!("[kernel] Hello world from kernel!");
        let pid = Process::current().pid();
        debug!("current pid = {}", pid);
    });
    info!(
        "[kxos-std/lib.rs] spawn kernel process, pid = {}",
        process.pid()
    );

    let hello_world_content = read_hello_world_content();
    let hello_world_filename = CString::new("hello_world").unwrap();
    let process = Process::spawn_user_process(hello_world_filename, hello_world_content);
    info!(
        "[kxos-std/lib.rs] spwan hello world process, pid = {}",
        process.pid()
    );

    let hello_c_content = read_hello_c_content();
    // glibc requires the filename starts as "/"
    let hello_c_filename = CString::new("/hello_c").unwrap();
    let process = Process::spawn_user_process(hello_c_filename, hello_c_content);
    info!("spawn hello_c process, pid = {}", process.pid());

    let fork_content = read_fork_content();
    let fork_filename = CString::new("fork").unwrap();
    let process = Process::spawn_user_process(fork_filename, fork_content);
    info!(
        "[kxos-std/lib.rs] spawn fork process, pid = {}",
        process.pid()
    );

    loop {
        // We don't have preemptive scheduler now.
        // The long running init process should yield its own execution to allow other tasks to go on.
        Process::yield_now();
    }
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

fn read_hello_c_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/hello_c/hello")
}
