//! The std library of jinux
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
#![feature(extend_one)]
// FIXME: This feature is used to support vm capbility now as a work around.
// Since this is an incomplete feature, use this feature is unsafe.
// We should find a proper method to replace this feature with min_specialization, which is a sound feature.
#![feature(specialization)]
#![feature(fn_traits)]

use crate::{
    prelude::*,
    user_apps::{get_busybox_app, UserApp},
};
use jinux_frame::{info, println};
use process::Process;

use crate::{
    process::{
        process_filter::ProcessFilter,
        wait::{wait_child_exit, WaitOptions},
    },
    user_apps::get_all_apps,
};

extern crate alloc;
extern crate lru;

pub mod driver;
pub mod error;
pub mod fs;
pub mod prelude;
mod process;
pub mod rights;
pub mod syscall;
pub mod tty;
mod user_apps;
mod util;
pub mod vm;
#[macro_use]
extern crate pod_derive;

pub fn init() {
    jinux_frame::disable_interrupts();
    driver::init();
    process::fifo_scheduler::init();
    jinux_frame::enable_interrupts();
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
        "[jinux-std/lib.rs] spawn kernel process, pid = {}",
        process.pid()
    );

    // FIXME: should be running this apps before we running shell?
    println!("");
    println!("[kernel] Running test programs");
    println!("");
    // Run test apps
    for app in get_all_apps().into_iter() {
        let UserApp {
            app_name,
            app_content,
            argv,
            envp,
        } = app;
        info!("[jinux-std/lib.rs] spwan {:?} process", app_name);
        Process::spawn_user_process(app_name.clone(), app_content, argv, Vec::new());
    }

    // Run busybox ash
    let UserApp {
        app_name,
        app_content,
        argv,
        envp,
    } = get_busybox_app();
    println!("");
    println!("BusyBox v1.35.0 built-in shell (ash)\n");
    Process::spawn_user_process(app_name.clone(), app_content, argv, Vec::new());

    loop {
        // We don't have preemptive scheduler now.
        // The long running init process should yield its own execution to allow other tasks to go on.
        // The init process should wait and reap all children.
        let _ = wait_child_exit(ProcessFilter::Any, WaitOptions::empty());
    }
}

/// first process never return
pub fn run_first_process() -> ! {
    Process::spawn_kernel_process(init_process);
    unreachable!()
}
