// SPDX-License-Identifier: MPL-2.0

//! The std library of Asterinas.
#![no_std]
#![deny(unsafe_code)]
#![allow(incomplete_features)]
#![feature(btree_cursors)]
#![feature(btree_extract_if)]
#![feature(const_option)]
#![feature(extend_one)]
#![feature(fn_traits)]
#![feature(format_args_nl)]
#![feature(int_roundings)]
#![feature(iter_repeat_n)]
#![feature(let_chains)]
#![feature(linked_list_remove)]
#![feature(register_tool)]
// FIXME: This feature is used to support vm capbility now as a work around.
// Since this is an incomplete feature, use this feature is unsafe.
// We should find a proper method to replace this feature with min_specialization, which is a sound feature.
#![feature(specialization)]
#![feature(step_trait)]
#![feature(trait_alias)]
#![feature(trait_upcasting)]
#![register_tool(component_access_control)]

use ostd::{
    arch::qemu::{exit_qemu, QemuExitCode},
    boot,
};
use process::Process;

use crate::{
    prelude::*,
    thread::{
        kernel_thread::{KernelThreadExt, ThreadOptions},
        Thread,
    },
};

extern crate alloc;
extern crate lru;
#[macro_use]
extern crate controlled;
#[macro_use]
extern crate getset;

pub mod arch;
pub mod console;
pub mod cpu;
pub mod device;
pub mod driver;
pub mod error;
pub mod events;
pub mod fs;
pub mod net;
pub mod prelude;
mod process;
mod sched;
pub mod softirq_id;
pub mod syscall;
mod taskless;
pub mod thread;
pub mod time;
mod util;
pub(crate) mod vdso;
pub mod vm;

pub fn init() {
    util::random::init();
    driver::init();
    time::init();
    net::init();
    sched::init();
    fs::rootfs::init(boot::initramfs()).unwrap();
    device::init().unwrap();
    vdso::init();
    taskless::init();
    process::init();
}

fn init_thread() {
    println!(
        "[kernel] Spawn init thread, tid = {}",
        current_thread!().tid()
    );
    // Work queue should be initialized before interrupt is enabled,
    // in case any irq handler uses work queue as bottom half
    thread::work_queue::init();
    // FIXME: Remove this if we move the step of mounting
    // the filesystems to be done within the init process.
    ostd::trap::enable_local();
    net::lazy_init();
    fs::lazy_init();
    // driver::pci::virtio::block::block_device_test();
    let thread = Thread::spawn_kernel_thread(ThreadOptions::new(|| {
        println!("[kernel] Hello world from kernel!");
        let current = current_thread!();
        let tid = current.tid();
        debug!("current tid = {}", tid);
    }));
    thread.join();
    info!(
        "[aster-nix/lib.rs] spawn kernel thread, tid = {}",
        thread.tid()
    );

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
   _   ___ _____ ___ ___ ___ _  _   _   ___
  /_\ / __|_   _| __| _ \_ _| \| | /_\ / __|
 / _ \\__ \ | | | _||   /| || .` |/ _ \\__ \
/_/ \_\___/ |_| |___|_|_\___|_|\_/_/ \_\___/
"
    );
    println!("\x1B[0m");
}
