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
#![feature(let_chains)]
// FIXME: This feature is used to support vm capbility now as a work around.
// Since this is an incomplete feature, use this feature is unsafe.
// We should find a proper method to replace this feature with min_specialization, which is a sound feature.
#![feature(specialization)]
#![feature(fn_traits)]
#![feature(linked_list_remove)]
#![feature(trait_alias)]
#![feature(register_tool)]
#![feature(trait_upcasting)]
#![register_tool(component_access_control)]

use crate::{
    prelude::*,
    process::status::ProcessStatus,
    thread::{kernel_thread::KernelThreadExt, Thread},
};
use jinux_frame::{boot, exit_qemu};
use process::Process;

extern crate alloc;
extern crate lru;
#[macro_use]
extern crate controlled;

pub mod device;
pub mod driver;
pub mod error;
pub mod events;
pub mod fs;
pub mod net;
pub mod prelude;
mod process;
pub mod syscall;
pub mod thread;
pub mod time;
mod util;
pub mod vm;

pub fn init() {
    driver::init();
    net::init();
    process::fifo_scheduler::init();
    fs::initramfs::init(boot::initramfs()).unwrap();
    device::init().unwrap();
}

fn init_thread() {
    println!(
        "[kernel] Spawn init thread, tid = {}",
        current_thread!().tid()
    );
    net::lazy_init();
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

    print_banner();
    let busybox = run_busybox().expect("run busybox fails");

    loop {
        // If busybox becomes zombie, then exit qemu.
        if *busybox.status().lock() == ProcessStatus::Zombie {
            println!("Exit jinux.");
            exit_qemu(jinux_frame::QemuExitCode::Success);
        }
        // We don't have preemptive scheduler now.
        // The long running init thread should yield its own execution to allow other tasks to go on.
        Thread::yield_now();
    }
}

/// first process never return
#[controlled]
pub fn run_first_process() -> ! {
    Thread::spawn_kernel_thread(init_thread);
    unreachable!()
}

fn run_busybox() -> Result<Arc<Process>> {
    let executable_path = "/usr/bin/busybox";
    let argv = ["sh", "-l"];
    let envp = [
        "SHELL=/bin/sh",
        "LOGNAME=root",
        "HOME=/",
        "USER=root",
        "PATH=/bin",
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
    Process::spawn_user_process(executable_path, argv, envp)
}

fn print_banner() {
    println!("\x1B[36m");
    println!(
        r"
       __   __  .__   __.  __    __  ___   ___ 
      |  | |  | |  \ |  | |  |  |  | \  \ /  / 
      |  | |  | |   \|  | |  |  |  |  \  V  /  
.--.  |  | |  | |  . `  | |  |  |  |   >   <   
|  `--'  | |  | |  |\   | |  `--'  |  /  .  \  
 \______/  |__| |__| \__|  \______/  /__/ \__\                                                                                            
"
    );
    println!("\x1B[0m");
}
