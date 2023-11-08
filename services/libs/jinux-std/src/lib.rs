//! The std library of jinux
#![no_std]
#![forbid(unsafe_code)]
#![allow(dead_code)]
#![allow(incomplete_features)]
#![allow(unused_variables)]
#![feature(exclusive_range_pattern)]
#![feature(btree_extract_if)]
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
#![feature(format_args_nl)]
#![register_tool(component_access_control)]

use crate::{
    prelude::*,
    thread::{
        kernel_thread::{KernelThreadExt, ThreadOptions},
        Thread,
    },
};
use jinux_frame::{
    arch::qemu::{exit_qemu, QemuExitCode},
    boot,
};
use process::Process;

extern crate alloc;
extern crate lru;
#[macro_use]
extern crate controlled;
#[macro_use]
extern crate ktest;

pub mod console;
pub mod device;
pub mod driver;
pub mod error;
pub mod events;
pub mod fs;
pub mod net;
pub mod prelude;
mod process;
mod sched;
pub mod syscall;
pub mod thread;
pub mod time;
mod util;
pub mod vm;

pub fn init() {
    driver::init();
    net::init();
    sched::init();
    fs::rootfs::init(boot::initramfs()).unwrap();
    device::init().unwrap();
}

fn init_thread() {
    println!(
        "[kernel] Spawn init thread, tid = {}",
        current_thread!().tid()
    );
    net::lazy_init();
    // driver::pci::virtio::block::block_device_test();
    let thread = Thread::spawn_kernel_thread(ThreadOptions::new(|| {
        println!("[kernel] Hello world from kernel!");
        let current = current_thread!();
        let tid = current.tid();
        debug!("current tid = {}", tid);
    }));
    thread.join();
    info!(
        "[jinux-std/lib.rs] spawn kernel thread, tid = {}",
        thread.tid()
    );
    thread::work_queue::init();

    print_banner();

    let karg = boot::kernel_cmdline();

    let initproc = Process::spawn_user_process(
        karg.get_initproc_path().unwrap(),
        karg.get_initproc_argv().to_vec(),
        karg.get_initproc_envp().to_vec(),
    )
    .expect("Run init process failed.");

    // Wait till initproc become zombie.
    while !initproc.is_zombie() {
        // We don't have preemptive scheduler now.
        // The long running init thread should yield its own execution to allow other tasks to go on.
        Thread::yield_now();
    }

    // TODO: exit via qemu isa debug device should not be the only way.
    let exit_code = if initproc.exit_code().unwrap() == 0 {
        QemuExitCode::Success
    } else {
        QemuExitCode::Failed
    };
    exit_qemu(exit_code);
}

/// first process never return
#[controlled]
pub fn run_first_process() -> ! {
    Thread::spawn_kernel_thread(ThreadOptions::new(init_thread));
    unreachable!()
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
