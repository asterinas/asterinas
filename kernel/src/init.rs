// SPDX-License-Identifier: MPL-2.0

//! Kernel initialization.

use core::{
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_cmdline::{INIT_PROC_ARGS, parse::ParamStorage};
use component::InitStage;
use ostd::{cpu::CpuId, util::id_set::Id};
use spin::once::Once;

use crate::{
    fs::{
        rootfs::{self, RootFsType},
        vfs::path::{FsPath, MountNamespace, PathResolver, PerMountFlags},
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

enum BootInit {
    Initramfs(&'static str),
    RootFs {
        mnt_ns: Arc<MountNamespace>,
        init_path: Option<&'static str>,
    },
}

// The main function of the first (non-idle) kernel thread
fn first_kthread() {
    println!("Spawn the first kernel thread");

    let init_mnt_ns = MountNamespace::get_init_singleton();
    let fs_resolver = init_mnt_ns.new_path_resolver();
    init_in_first_kthread(&fs_resolver);
    let boot_init = prepare_boot_init(&fs_resolver);

    print_banner();

    INIT_PROCESS.call_once(|| {
        let karg = INIT_PROC_ARGS.get().unwrap();
        let argv = karg.argv().to_vec();
        let envp = karg.envp().to_vec();
        match boot_init {
            BootInit::Initramfs(init_path) => {
                println!("[kernel] running {} as the initramfs init", init_path);
                spawn_init_process(init_mnt_ns.clone(), init_path, argv, envp)
            }
            BootInit::RootFs {
                mnt_ns,
                init_path: Some(init_path),
            } => {
                println!("[kernel] running {} as the rootfs init", init_path);
                spawn_init_process(mnt_ns, init_path, argv, envp)
            }
            BootInit::RootFs {
                mnt_ns,
                init_path: None,
            } => spawn_default_rootfs_init(mnt_ns, argv, envp),
        }
        .expect("failed to run the init process")
    });
}

fn prepare_boot_init(path_resolver: &PathResolver) -> BootInit {
    if let Some(init_path) = find_initramfs_init(path_resolver) {
        return BootInit::Initramfs(init_path);
    }

    let mnt_ns = mount_rootfs();
    let init_path = INIT_PATH.get().map(String::as_str);
    BootInit::RootFs { mnt_ns, init_path }
}

fn find_initramfs_init(path_resolver: &PathResolver) -> Option<&'static str> {
    const DEFAULT_INITRAMFS_INIT_PATH: &str = "/init";

    if let Some(init_path) = RDINIT_PATH.get().map(String::as_str) {
        let lookup_result = FsPath::try_from(init_path)
            .and_then(|fs_path| path_resolver.lookup(&fs_path).map(|_| ()));
        if let Err(error) = lookup_result {
            warn!(
                "check access for rdinit={} failed: {:?}, ignoring",
                init_path, error
            );
            return None;
        }

        return Some(init_path);
    }

    Some(DEFAULT_INITRAMFS_INIT_PATH).filter(|init_path| {
        path_resolver
            .lookup(&FsPath::try_from(*init_path).unwrap())
            .is_ok()
    })
}

fn mount_rootfs() -> Arc<MountNamespace> {
    let root = ROOT_PATH
        .get()
        .expect("neither an initramfs init nor root= was provided");
    let rootfs_types = ROOTFS_TYPES
        .get()
        .map(|rootfs_types| rootfs_types.as_slice())
        .unwrap_or(RootFsType::ALL);

    rootfs::mount(root, rootfs_types, root_mount_flags())
        .expect("failed to mount the root filesystem")
}

fn spawn_default_rootfs_init(
    mnt_ns: Arc<MountNamespace>,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<Arc<Process>> {
    // Linux probes the fallback init executables in this order:
    // <https://elixir.bootlin.com/linux/v6.19/source/init/main.c#L1604-L1607>.
    const DEFAULT_INIT_EXEC_PATHS: &[&str] = &["/sbin/init", "/etc/init", "/bin/init", "/bin/sh"];

    let mut last_error = None;

    for &init_path in DEFAULT_INIT_EXEC_PATHS {
        // FIXME: Avoid cloning `argv` and `envp` for each fallback candidate.
        match spawn_init_process(mnt_ns.clone(), init_path, argv.clone(), envp.clone()) {
            Ok(process) => {
                println!("[kernel] running {} as the rootfs init", init_path);
                return Ok(process);
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap())
}

static INIT_PROCESS: Once<Arc<Process>> = Once::new();

fn root_mount_flags() -> PerMountFlags {
    let mut flags = PerMountFlags::default();
    if ROOT_MOUNT_READ_ONLY.load(Ordering::Relaxed) {
        flags.insert(PerMountFlags::RDONLY);
    }
    flags
}

struct SetRootMountReadOnly;

impl ParamStorage for SetRootMountReadOnly {
    type Value = bool;

    fn store_param(&self, value: Self::Value) {
        if value {
            ROOT_MOUNT_READ_ONLY.store(true, Ordering::Relaxed);
        }
    }
}

struct SetRootMountReadWrite;

impl ParamStorage for SetRootMountReadWrite {
    type Value = bool;

    fn store_param(&self, value: Self::Value) {
        if value {
            ROOT_MOUNT_READ_ONLY.store(false, Ordering::Relaxed);
        }
    }
}

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

pub(super) fn on_first_process_startup(ctx: &Context) {
    component::init_all(InitStage::Process, component::parse_metadata!()).unwrap();
    crate::device::init_in_first_process(ctx).unwrap();
    crate::fs::init_in_first_process(ctx);
}

static RDINIT_PATH: Once<String> = Once::new();
aster_cmdline::define_kv_param!("rdinit", RDINIT_PATH);

static ROOT_PATH: Once<String> = Once::new();
aster_cmdline::define_kv_param!("root", ROOT_PATH);

/// Root filesystem type candidates.
struct RootFsTypes(Vec<RootFsType>);

impl RootFsTypes {
    /// Returns the root filesystem type candidates as a slice.
    fn as_slice(&self) -> &[RootFsType] {
        self.0.as_slice()
    }
}

impl FromStr for RootFsTypes {
    type Err = core::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let candidates = value
            .split(',')
            .filter(|type_name| !type_name.is_empty())
            .map(str::parse)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self(candidates))
    }
}

static ROOTFS_TYPES: Once<RootFsTypes> = Once::new();
aster_cmdline::define_kv_param!("rootfstype", ROOTFS_TYPES);

static INIT_PATH: Once<String> = Once::new();
aster_cmdline::define_kv_param!("init", INIT_PATH);

static ROOT_MOUNT_READ_ONLY: AtomicBool = AtomicBool::new(true);
static RO_PARAM: SetRootMountReadOnly = SetRootMountReadOnly;
aster_cmdline::define_flag_param!("ro", RO_PARAM);
static RW_PARAM: SetRootMountReadWrite = SetRootMountReadWrite;
aster_cmdline::define_flag_param!("rw", RW_PARAM);
