// SPDX-License-Identifier: MPL-2.0

//! Kernel initialization.

use aster_cmdline::INIT_PROC_ARGS;
use component::InitStage;
use ostd::{cpu::CpuId, util::id_set::Id};
use spin::once::Once;

use crate::{
    fs::{
        initramfs, rootfs,
        vfs::path::{FsPath, MountNamespace, Path, PathResolver},
    },
    prelude::*,
    process::{Process, spawn_init_process},
    sched::SchedPolicy,
    thread::kernel_thread::ThreadOptions,
};

pub(super) fn main() {
    // Initialize the global states for all CPUs.
    ostd::early_println!("OSTD initialized. Preparing components.");
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

pub(super) fn on_first_process_startup(ctx: &Context) {
    component::init_all(InitStage::Process, component::parse_metadata!()).unwrap();
    crate::device::init_in_first_process(ctx).unwrap();
    crate::fs::init_in_first_process(ctx);
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
    ostd::info!("Idle thread for CPU #0 started");

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

    panic!(
        "The init process terminates with code {:?}",
        init_process.status().exit_code()
    );
}

fn ap_idle_loop() {
    ostd::info!(
        "Idle thread for CPU #{} started",
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
    println!("Spawn the first kernel thread");

    let init_mnt_ns = MountNamespace::get_init_singleton();
    let fs_resolver = init_mnt_ns.new_path_resolver();
    init_in_first_kthread(&fs_resolver);
    let boot_init = prepare_boot_init(fs_resolver);

    print_banner();

    INIT_PROCESS.call_once(|| {
        let karg = INIT_PROC_ARGS.get().unwrap();
        let argv = karg.argv().to_vec();
        let envp = karg.envp().to_vec();
        boot_init
            .spawn(argv, envp)
            .expect("failed to run the init process")
    });
}

struct BootInit {
    path_resolver: PathResolver,
    init_path: Option<(Path, &'static str)>,
}

impl BootInit {
    fn spawn(self, argv: Vec<CString>, envp: Vec<CString>) -> Result<Arc<Process>> {
        let Self {
            path_resolver,
            init_path,
        } = self;

        // Only rootfs boot may have no init path; initramfs boot always provides one.
        let Some((init_path, init_name)) = init_path else {
            return spawn_default_rootfs_init(path_resolver, argv, envp);
        };

        println!("[kernel] running {} as the init process", init_name);
        spawn_init_process(
            path_resolver,
            init_path,
            with_init_argv0(init_name, argv),
            envp,
        )
    }
}

fn prepare_boot_init(mut path_resolver: PathResolver) -> BootInit {
    if let Ok(init_path) = initramfs::find_init(&path_resolver) {
        return BootInit {
            path_resolver,
            init_path: Some(init_path),
        };
    }

    rootfs::switch_to_rootfs(&mut path_resolver)
        .expect("neither an initramfs init nor a usable root filesystem was available");
    let init_path = rootfs::find_init(&path_resolver).expect("failed to resolve rootfs init path");
    BootInit {
        path_resolver,
        init_path,
    }
}

fn spawn_default_rootfs_init(
    path_resolver: PathResolver,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Process>> {
    // Linux probes the fallback init executables in this order:
    // Reference: <https://elixir.bootlin.com/linux/v6.19/source/init/main.c#L1634>.
    const DEFAULT_INIT_EXEC_PATHS: &[&str] = &["/sbin/init", "/etc/init", "/bin/init", "/bin/sh"];

    let mut last_error = None;

    for &init_name in DEFAULT_INIT_EXEC_PATHS {
        // FIXME: Avoid cloning `argv` and `envp` for each fallback candidate.
        let init_path = match path_resolver.lookup(&FsPath::try_from(init_name).unwrap()) {
            Ok(path) => path,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };

        match spawn_init_process(
            path_resolver.clone(),
            init_path,
            with_init_argv0(init_name, argv.clone()),
            envp.clone(),
        ) {
            Ok(process) => {
                println!("[kernel] running {} as the rootfs init", init_name);
                return Ok(process);
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap())
}

fn with_init_argv0(init_name: &str, mut argv: Vec<CString>) -> Vec<CString> {
    // Linux prepends the init executable path as `argv[0]`.
    // Reference: <https://elixir.bootlin.com/linux/v6.19/source/init/main.c#L1491>.
    argv.insert(0, CString::new(init_name).unwrap());
    argv
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
    #[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
    crate::vdso::init_in_first_kthread();
}

fn print_banner() {
    println!("");
    println!("{}", logo_ascii_art::get_gradient_color_version());
}
