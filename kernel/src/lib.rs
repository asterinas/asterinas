// SPDX-License-Identifier: MPL-2.0

//! Aster-nix is the Asterinas kernel, a safe, efficient unix-like
//! operating system kernel built on top of OSTD and OSDK.

#![no_std]
#![no_main]
#![deny(unsafe_code)]
#![feature(btree_cursors)]
#![feature(btree_extract_if)]
#![feature(debug_closure_helpers)]
#![feature(extract_if)]
#![feature(format_args_nl)]
#![feature(integer_sign_cast)]
#![feature(let_chains)]
#![feature(linked_list_cursors)]
#![feature(linked_list_retain)]
#![feature(negative_impls)]
#![feature(panic_can_unwind)]
#![feature(register_tool)]
#![feature(min_specialization)]
#![feature(trait_alias)]
#![feature(trait_upcasting)]
#![feature(associated_type_defaults)]
#![register_tool(component_access_control)]

use component::InitStage;
use ostd::{
    arch::qemu::{exit_qemu, QemuExitCode},
    boot::boot_info,
    cpu::CpuId,
    util::id_set::Id,
};
use spin::once::Once;

use crate::{
    fs::{fs_resolver::FsResolver, path::MountNamespace},
    kcmdline::KCmdlineArg,
    prelude::*,
    process::{spawn_init_process, Process},
    sched::SchedPolicy,
    thread::kernel_thread::ThreadOptions,
};

extern crate alloc;
extern crate lru;
#[macro_use]
extern crate controlled;
#[macro_use]
extern crate getset;

#[cfg_attr(target_arch = "x86_64", path = "arch/x86/mod.rs")]
#[cfg_attr(target_arch = "riscv64", path = "arch/riscv/mod.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "arch/loongarch/mod.rs")]
mod arch;

mod context;
mod cpu;
mod device;
mod driver;
mod error;
mod events;
mod fs;
mod ipc;
mod kcmdline;
mod net;
mod prelude;
mod process;
mod sched;
mod security;
mod syscall;
mod thread;
mod time;
mod util;
// TODO: Add vDSO support for other architectures.
#[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
mod vdso;
mod vm;

#[ostd::main]
#[controlled]
fn main() {
    // Initializes the global states for all CPUs.
    ostd::early_println!("[kernel] OSTD initialized. Preparing components.");
    component::init_all(InitStage::Bootstrap, component::parse_metadata!()).unwrap();
    init();

    // Initializes the per-CPU states for BSP.
    init_on_each_cpu();

    // Enable APs.
    ostd::boot::smp::register_ap_entry(ap_init);

    // Give the control of the BSP to the idle thread.
    ThreadOptions::new(bsp_idle_loop)
        .cpu_affinity(CpuId::bsp().into())
        .sched_policy(SchedPolicy::Idle)
        .spawn();
}

fn init() {
    thread::init();
    util::random::init();
    driver::init();
    time::init();
    net::init();
    sched::init();
    process::init();
    fs::init();
    security::init();
}

fn init_on_each_cpu() {
    sched::init_on_each_cpu();
    process::init_on_each_cpu();
    fs::init_on_each_cpu();
    time::init_on_each_cpu();
}

fn ap_init() {
    init_on_each_cpu();

    ThreadOptions::new(ap_idle_loop)
        // No races because `ap_init` runs on a certain AP.
        .cpu_affinity(CpuId::current_racy().into())
        .sched_policy(SchedPolicy::Idle)
        .spawn();
}

//-----------------------------------------------------------------------------
// The first kernel thread
//-----------------------------------------------------------------------------

// The main function of the first (non-idle) kernel thread
fn first_kthread() {
    println!("[kernel] Spawn the first kernel thread");

    let init_mnt_ns = MountNamespace::get_init_singleton();
    let fs_resolver = init_mnt_ns.new_fs_resolver();
    init_in_first_kthread(&fs_resolver);

    print_banner();

    INIT_PROCESS.call_once(|| {
        let karg: KCmdlineArg = boot_info().kernel_cmdline.as_str().into();
        spawn_init_process(
            karg.get_initproc_path().unwrap(),
            karg.get_initproc_argv().to_vec(),
            karg.get_initproc_envp().to_vec(),
        )
        .expect("Run init process failed.")
    });
}

static INIT_PROCESS: Once<Arc<Process>> = Once::new();

fn init_in_first_kthread(fs_resolver: &FsResolver) {
    component::init_all(InitStage::Kthread, component::parse_metadata!()).unwrap();
    // Work queue should be initialized before interrupt is enabled,
    // in case any irq handler uses work queue as bottom half
    thread::work_queue::init_in_first_kthread();
    net::init_in_first_kthread();
    fs::init_in_first_kthread(fs_resolver);
    ipc::init_in_first_kthread();
    #[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
    vdso::init_in_first_kthread();
}

fn init_in_first_process(ctx: &Context) {
    component::init_all(InitStage::Process, component::parse_metadata!()).unwrap();
    device::init_in_first_process(ctx).unwrap();
    fs::init_in_first_process(ctx);
    process::init_in_first_process(ctx);
}

fn print_banner() {
    println!("");
    println!("{}", logo_ascii_art::get_gradient_color_version());
}

//-----------------------------------------------------------------------------
// Per-CPU idle threads
//-----------------------------------------------------------------------------

// Note: Keep the code in the idle loop to the bare minimum.
//
// We do not want the idle loop to
// rely on the APIs of other kernel subsystems for two reasons.
// First, the idle task must never sleep or block.
// This property is relied upon by the scheduler.
// Second, the idle task is spawned before the kernel is fully initialized.
// So other subsystems may not be ready, yet.
//
// In addition,
// doing more work in the idle task may have negative impact on
// the latency to switching from the idle task to a useful, runnable one.

fn bsp_idle_loop() {
    ostd::early_println!("[kernel] Idle thread for CPU #0 started");

    // Spawn the first non-idle kernel thread on BSP.
    ThreadOptions::new(first_kthread)
        .cpu_affinity(CpuId::bsp().into())
        .sched_policy(SchedPolicy::default())
        .spawn();

    // Wait till the init process is spawned.
    let init_process = loop {
        if let Some(init_process) = INIT_PROCESS.get() {
            break init_process;
        };

        ostd::task::halt_cpu();
    };

    // Wait till the init process becomes zombie.
    while !init_process.status().is_zombie() {
        ostd::task::halt_cpu();
    }

    // TODO: exit via qemu isa debug device should not be the only way.
    let exit_code = if init_process.status().exit_code() == 0 {
        QemuExitCode::Success
    } else {
        QemuExitCode::Failed
    };
    exit_qemu(exit_code);
}

fn ap_idle_loop() {
    log::info!(
        "[kernel] Idle thread for CPU #{} started",
        // No races because this function runs on a certain AP.
        CpuId::current_racy().as_usize(),
    );

    loop {
        ostd::task::halt_cpu();
    }
}
