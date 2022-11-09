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
#![feature(const_option)]

use crate::prelude::*;
use kxos_frame::{info, println};
use process::Process;

use crate::{
    process::{
        process_filter::ProcessFilter,
        wait::{wait_child_exit, WaitOptions},
    },
    user_apps::get_all_apps,
};

extern crate alloc;

pub mod driver;
pub mod error;
pub mod fs;
mod memory;
pub mod prelude;
mod process;
pub mod rights;
pub mod syscall;
mod user_apps;
mod util;
pub mod vm;
#[macro_use]
extern crate kxos_frame_pod_derive;

pub fn init() {
    driver::init();
    process::fifo_scheduler::init();
}

pub fn init_process() {
    println!("[kernel] Spawn init process!, pid = {}", current!().pid());
    driver::pci::virtio::block::block_device_test();
    let process = Process::spawn_kernel_process(|| {
        println!("[kernel] Hello world from kernel!");
        let current = current!();
        let pid = current.pid();
        info!("current pid = {}", pid);
        let ppid = current.parent().unwrap().pid();
        info!("current ppid = {}", ppid);
    });
    info!(
        "[kxos-std/lib.rs] spawn kernel process, pid = {}",
        process.pid()
    );

    for app in get_all_apps() {
        let app_name = app.app_name();
        info!("[kxos-std/lib.rs] spwan {:?} process", app.app_name());
        let argv = vec![app_name.clone()];
        let process = Process::spawn_user_process(app_name, app.app_content(), argv, Vec::new());
        info!(
            "[kxos-std/lib.rs] {:?} process exits, pid = {}",
            app.app_name(),
            process.pid()
        );
    }

    loop {
        // We don't have preemptive scheduler now.
        // The long running init process should yield its own execution to allow other tasks to go on.
        // The init process should wait and reap all children.
        let _ = wait_child_exit(ProcessFilter::Any, WaitOptions::empty());
    }
}

/// first process never return
pub fn run_first_process() -> ! {
    // let elf_file_content = read_hello_world_content();
    Process::spawn_kernel_process(init_process);
    unreachable!()
}
