// SPDX-License-Identifier: MPL-2.0

//! Aster-nix is the Asterinas kernel, a safe, efficient unix-like
//! operating system kernel built on top of OSTD and OSDK.

#![no_std]
#![no_main]
#![deny(unsafe_code)]
#![expect(incomplete_features)]
#![feature(btree_cursors)]
#![feature(btree_extract_if)]
#![feature(debug_closure_helpers)]
#![feature(extend_one)]
#![feature(fn_traits)]
#![feature(format_args_nl)]
#![feature(int_roundings)]
#![feature(let_chains)]
#![feature(linked_list_cursors)]
#![feature(linked_list_remove)]
#![feature(linked_list_retain)]
#![feature(negative_impls)]
#![feature(panic_can_unwind)]
#![feature(register_tool)]
// FIXME: This feature is used to support vm capbility now as a work around.
// Since this is an incomplete feature, use this feature is unsafe.
// We should find a proper method to replace this feature with min_specialization, which is a sound feature.
#![feature(specialization)]
#![feature(step_trait)]
#![feature(trait_alias)]
#![feature(trait_upcasting)]
#![register_tool(component_access_control)]

use kcmdline::KCmdlineArg;
use ostd::{
    arch::qemu::{exit_qemu, QemuExitCode},
    boot::boot_info,
    cpu::{CpuId, CpuSet, PinCurrentCpu},
};
use process::Process;
use sched::SchedPolicy;

use crate::{prelude::*, thread::kernel_thread::ThreadOptions};

extern crate alloc;
extern crate lru;
#[macro_use]
extern crate controlled;
#[macro_use]
extern crate getset;

pub mod arch;
pub mod context;
pub mod cpu;
pub mod device;
pub mod driver;
pub mod error;
pub mod events;
pub mod fs;
pub mod ipc;
pub mod kcmdline;
pub mod net;
pub mod prelude;
mod process;
mod sched;
pub mod syscall;
pub mod thread;
pub mod time;
mod util;
pub(crate) mod vdso;
pub mod vm;

#[ostd::main]
#[controlled]
pub fn main() {
    ostd::early_println!("[kernel] OSTD initialized. Preparing components.");
    component::init_all(component::parse_metadata!()).unwrap();
    init();

    // Spawn all AP idle threads.
    ostd::boot::smp::register_ap_entry(ap_init);

    // Spawn the first kernel thread on BSP.
    let mut affinity = CpuSet::new_empty();
    affinity.add(CpuId::bsp());
    ThreadOptions::new(init_thread)
        .cpu_affinity(affinity)
        .sched_policy(SchedPolicy::Idle)
        .spawn();
}

pub fn init() {
    thread::init();
    util::random::init();
    driver::init();
    time::init();
    #[cfg(target_arch = "x86_64")]
    net::init();
    sched::init();
    fs::rootfs::init(boot_info().initramfs.expect("No initramfs found!")).unwrap();
    device::init().unwrap();
    syscall::init();
    vdso::init();
    process::init();
}

fn ap_init() {
    fn ap_idle_thread() {
        let preempt_guard = ostd::task::disable_preempt();
        let cpu_id = preempt_guard.current_cpu();
        drop(preempt_guard);
        log::info!("Kernel idle thread for CPU #{} started.", cpu_id.as_usize());

        loop {
            crate::thread::Thread::yield_now();
            ostd::cpu::sleep_for_interrupt();
        }
    }
    let preempt_guard = ostd::task::disable_preempt();
    let cpu_id = preempt_guard.current_cpu();
    drop(preempt_guard);

    ThreadOptions::new(ap_idle_thread)
        .cpu_affinity(cpu_id.into())
        .sched_policy(SchedPolicy::Idle)
        .spawn();
}

fn init_thread() {
    println!("[kernel] Spawn init thread");
    // Work queue should be initialized before interrupt is enabled,
    // in case any irq handler uses work queue as bottom half
    thread::work_queue::init();
    #[cfg(target_arch = "x86_64")]
    net::lazy_init();
    fs::lazy_init();
    ipc::init();
    // driver::pci::virtio::block::block_device_test();
    let thread = ThreadOptions::new(|| {
        println!("[kernel] Hello world from kernel!");
    })
    .spawn();
    thread.join();

    print_banner();

    let karg: KCmdlineArg = boot_info().kernel_cmdline.as_str().into();

    let initproc = Process::spawn_user_process(
        karg.get_initproc_path().unwrap(),
        karg.get_initproc_argv().to_vec(),
        karg.get_initproc_envp().to_vec(),
    )
    .expect("Run init process failed.");
    // Wait till initproc become zombie.
    while !initproc.status().is_zombie() {
        crate::thread::Thread::yield_now();
        ostd::cpu::sleep_for_interrupt();
    }

    // TODO: exit via qemu isa debug device should not be the only way.
    let exit_code = if initproc.status().exit_code() == 0 {
        QemuExitCode::Success
    } else {
        QemuExitCode::Failed
    };
    exit_qemu(exit_code);
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
