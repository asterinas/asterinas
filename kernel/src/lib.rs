// SPDX-License-Identifier: MPL-2.0

//! Aster-nix is the Asterinas kernel, a safe, efficient unix-like
//! operating system kernel built on top of OSTD and OSDK.

#![no_std]
#![no_main]
#![deny(unsafe_code)]
#![feature(btree_cursors)]
#![feature(btree_extract_if)]
#![feature(debug_closure_helpers)]
#![feature(extend_one)]
#![feature(extract_if)]
#![feature(fn_traits)]
#![feature(format_args_nl)]
#![feature(int_roundings)]
#![feature(integer_sign_cast)]
#![feature(let_chains)]
#![feature(linked_list_cursors)]
#![feature(linked_list_remove)]
#![feature(linked_list_retain)]
#![feature(negative_impls)]
#![feature(panic_can_unwind)]
#![feature(register_tool)]
#![feature(min_specialization)]
#![feature(step_trait)]
#![feature(trait_alias)]
#![feature(trait_upcasting)]
#![feature(associated_type_defaults)]
#![register_tool(component_access_control)]

use kcmdline::KCmdlineArg;
use ostd::{
    arch::qemu::{exit_qemu, QemuExitCode},
    boot::boot_info,
    cpu::{CpuId, CpuSet},
};
use process::{spawn_init_process, Process};
use sched::SchedPolicy;

use crate::{
    fs::fs_resolver::FsResolver, namespace::NsContext, prelude::*,
    thread::kernel_thread::ThreadOptions,
};

extern crate alloc;
extern crate lru;
#[macro_use]
extern crate controlled;
#[macro_use]
extern crate getset;

#[cfg(target_arch = "x86_64")]
#[path = "arch/x86/mod.rs"]
pub mod arch;
#[cfg(target_arch = "riscv64")]
#[path = "arch/riscv/mod.rs"]
pub mod arch;
#[cfg(target_arch = "loongarch64")]
#[path = "arch/loongarch/mod.rs"]
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
mod namespace;
pub mod net;
pub mod prelude;
mod process;
mod sched;
pub mod syscall;
pub mod thread;
pub mod time;
mod util;
// TODO: Add vDSO support for other architectures.
#[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
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
    ThreadOptions::new(first_kthread)
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
    process::init();
    fs::init();
    namespace::init();
}

fn init_in_first_kthread(fs_resolver: &FsResolver) {
    // Work queue should be initialized before interrupt is enabled,
    // in case any irq handler uses work queue as bottom half
    thread::work_queue::init_in_first_kthread();
    #[cfg(target_arch = "x86_64")]
    net::init_in_first_kthread();
    fs::init_in_first_kthread(fs_resolver);
    ipc::init_in_first_kthread();
}

fn init_in_first_process(ctx: &Context) {
    device::init_in_first_process(ctx).unwrap();
    fs::init_in_first_process(ctx);
    process::init_in_first_process(ctx);
    #[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
    vdso::init_in_first_process();
}

fn ap_init() {
    fn ap_idle_thread() {
        log::info!(
            "Kernel idle thread for CPU #{} started.",
            // No races because `ap_idle_thread` runs on a certain AP.
            CpuId::current_racy().as_usize(),
        );

        loop {
            ostd::task::halt_cpu();
        }
    }

    ThreadOptions::new(ap_idle_thread)
        // No races because `ap_init` runs on a certain AP.
        .cpu_affinity(CpuId::current_racy().into())
        .sched_policy(SchedPolicy::Idle)
        .spawn();
}

fn first_kthread() {
    println!("[kernel] Spawn init thread");

    let init_ns_context = NsContext::new_init();
    let fs_resolver = init_ns_context.mnt_ns().create_fs_resolver();

    init_in_first_kthread(&fs_resolver);

    print_banner();

    let karg: KCmdlineArg = boot_info().kernel_cmdline.as_str().into();

    let initproc = spawn_init_process(
        karg.get_initproc_path().unwrap(),
        karg.get_initproc_argv().to_vec(),
        karg.get_initproc_envp().to_vec(),
        init_ns_context,
    )
    .expect("Run init process failed.");

    // Wait till initproc become zombie.
    while !initproc.status().is_zombie() {
        ostd::task::halt_cpu();
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
