// SPDX-License-Identifier: MPL-2.0

//! Kernel initialization.

use aster_cmdline::KCMDLINE;
use component::InitStage;
use ostd::{cpu::CpuId, util::id_set::Id};
use spin::once::Once;

use crate::{
    fs::path::{MountNamespace, PathResolver},
    prelude::*,
    process::{Process, spawn_init_process},
    sched::SchedPolicy,
    thread::kernel_thread::ThreadOptions,
};

pub(super) fn main() {
    // Initialize the global states for all CPUs.
    ostd::early_println!("[kernel] OSTD initialized. Preparing components.");
    component::init_all(InitStage::Bootstrap, component::parse_metadata!()).unwrap();
    init();

    // Initialize the per-CPU states for BSP.
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
    crate::arch::init();
    crate::thread::init();
    crate::util::random::init();
    crate::driver::init();
    crate::time::init();
    crate::net::init();
    crate::sched::init();
    crate::process::init();
    crate::fs::init();
    crate::security::init();
}

fn init_on_each_cpu() {
    crate::sched::init_on_each_cpu();
    crate::process::init_on_each_cpu();
    crate::fs::init_on_each_cpu();
    crate::time::init_on_each_cpu();
}

fn ap_init() {
    // Initialize the per-CPU states for AP.
    init_on_each_cpu();

    ThreadOptions::new(ap_idle_loop)
        // No races because `ap_init` runs on a certain AP.
        .cpu_affinity(CpuId::current_racy().into())
        .sched_policy(SchedPolicy::Idle)
        .spawn();
}

//--------------------------------------------------------------------------
// Per-CPU idle threads
//--------------------------------------------------------------------------

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
    log::info!("[kernel] Idle thread for CPU #0 started");

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

    // According to the Linux implementation, we should panic once the init process exits.
    // Currently, we choose to power off the machine for more flexibility in testing with QEMU.
    let raw_exit_code = init_process.status().exit_code();
    let exit_code = if raw_exit_code == 0 {
        ostd::power::ExitCode::Success
    } else {
        ostd::power::ExitCode::Failure
    };
    ostd::power::poweroff(exit_code);
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

//--------------------------------------------------------------------------
// The first kernel thread
//--------------------------------------------------------------------------

// The main function of the first (non-idle) kernel thread
fn first_kthread() {
    println!("[kernel] Spawn the first kernel thread");

    let init_mnt_ns = MountNamespace::get_init_singleton();
    let fs_resolver = init_mnt_ns.new_path_resolver();
    init_in_first_kthread(&fs_resolver);

    print_banner();

    INIT_PROCESS.call_once(|| {
        let karg = KCMDLINE.get().unwrap();
        spawn_init_process(
            karg.get_initproc_path().unwrap(),
            karg.get_initproc_argv().to_vec(),
            karg.get_initproc_envp().to_vec(),
        )
        .expect("Run init process failed.")
    });
}

static INIT_PROCESS: Once<Arc<Process>> = Once::new();

fn init_in_first_kthread(path_resolver: &PathResolver) {
    component::init_all(InitStage::Kthread, component::parse_metadata!()).unwrap();
    // Work queue should be initialized before interrupt is enabled,
    // in case any irq handler uses work queue as bottom half
    crate::thread::work_queue::init_in_first_kthread();
    crate::device::init_in_first_kthread();
    crate::net::init_in_first_kthread();
    crate::fs::init_in_first_kthread(path_resolver);
    crate::ipc::init_in_first_kthread();
    #[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
    crate::vdso::init_in_first_kthread();
}

fn print_banner() {
    println!("");
    println!("{}", logo_ascii_art::get_gradient_color_version());
}

pub(crate) fn on_first_process_startup(ctx: &Context) {
    component::init_all(InitStage::Process, component::parse_metadata!()).unwrap();
    crate::device::init_in_first_process(ctx).unwrap();
    crate::fs::init_in_first_process(ctx);
    crate::process::init_in_first_process(ctx);
}
