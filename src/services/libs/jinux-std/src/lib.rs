//! The std library of jinux
#![no_std]
#![forbid(unsafe_code)]
#![allow(dead_code)]
#![allow(incomplete_features)]
#![allow(unused_variables)]
#![feature(cstr_from_bytes_until_nul)]
#![feature(exclusive_range_pattern)]
#![feature(btree_drain_filter)]
#![feature(const_option)]
#![feature(extend_one)]
// FIXME: This feature is used to support vm capbility now as a work around.
// Since this is an incomplete feature, use this feature is unsafe.
// We should find a proper method to replace this feature with min_specialization, which is a sound feature.
#![feature(specialization)]
#![feature(fn_traits)]
#![feature(linked_list_remove)]
#![feature(register_tool)]
#![register_tool(component_access_control)]

use crate::{
    prelude::*,
    thread::{kernel_thread::KernelThreadExt, Thread},
};
use process::Process;

extern crate alloc;
extern crate lru;
#[macro_use]
extern crate controlled;

pub mod driver;
pub mod error;
pub mod fs;
pub mod prelude;
mod process;
pub mod rights;
pub mod syscall;
pub mod thread;
pub mod time;
pub mod tty;
mod util;
pub mod vm;

pub fn init() {
    driver::init();
    process::fifo_scheduler::init();
    fs::initramfs::init(read_ramdisk_content()).unwrap();
}

fn init_thread() {
    println!(
        "[kernel] Spawn init thread, tid = {}",
        current_thread!().tid()
    );
    // driver::pci::virtio::block::block_device_test();
    let thread = Thread::spawn_kernel_thread(|| {
        println!("[kernel] Hello world from kernel!");
        let current = current_thread!();
        let tid = current.tid();
        debug!("current tid = {}", tid);
    });
    thread.join();
    info!(
        "[jinux-std/lib.rs] spawn kernel thread, tid = {}",
        thread.tid()
    );

    run_busybox().expect("run busybox fails");

    loop {
        // We don't have preemptive scheduler now.
        // The long running init thread should yield its own execution to allow other tasks to go on.
        Thread::yield_now();
    }
}

fn read_ramdisk_content() -> &'static [u8] {
    include_bytes!("../../../../ramdisk/build/ramdisk.cpio")
}

/// first process never return
#[controlled]
pub fn run_first_process() -> ! {
    Thread::spawn_kernel_thread(init_thread);
    unreachable!()
}

fn run_busybox() -> Result<()> {
    let executable_path = "/busybox/busybox";
    let argv = ["sh", "-l"];
    let envp = [
        "SHELL=/bin/sh",
        "PWD=/",
        "LOGNAME=root",
        "HOME=/",
        "USER=root",
        "PATH=/bin",
        "OLDPWD=/",
    ];
    let argv = argv
        .into_iter()
        .map(|arg| CString::new(arg).unwrap())
        .collect();
    let envp = envp
        .into_iter()
        .map(|env| CString::new(env).unwrap())
        .collect();
    println!("");
    println!("BusyBox v1.35.0 built-in shell (ash)\n");
    Process::spawn_user_process(executable_path, argv, envp);
    Ok(())
}
