// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

//! The architecture-independent boot module, which provides a universal interface
//! from the bootloader to the rest of OSTD.
//!

pub mod kcmdline;
pub mod memory_region;

use alloc::{boxed::Box, string::String, vec::Vec};

use kcmdline::KCmdlineArg;
use memory_region::MemoryRegion;
use spin::Once;

use crate::task::{set_scheduler, FifoScheduler, Scheduler, TaskOptions};

/// ACPI information from the bootloader.
///
/// The boot crate can choose either providing the raw RSDP physical address or
/// providing the RSDT/XSDT physical address after parsing RSDP.
/// This is because bootloaders differ in such behaviors.
#[derive(Copy, Clone, Debug)]
pub enum BootloaderAcpiArg {
    /// The bootloader does not provide one, a manual search is needed.
    NotProvided,
    /// Physical address of the RSDP.
    Rsdp(usize),
    /// Address of RSDT provided in RSDP v1.
    Rsdt(usize),
    /// Address of XSDT provided in RSDP v2+.
    Xsdt(usize),
}

/// The framebuffer arguments.
#[derive(Copy, Clone, Debug)]
pub struct BootloaderFramebufferArg {
    /// The address of the buffer.
    pub address: usize,
    /// The width of the buffer.
    pub width: usize,
    /// The height of the buffer.
    pub height: usize,
    /// Bits per pixel of the buffer.
    pub bpp: usize,
}

macro_rules! define_global_static_boot_arguments {
    ( $( $lower:ident, $upper:ident, $typ:ty; )* ) => {
        // Define statics and corresponding public getter APIs.
        $(
            static $upper: Once<$typ> = Once::new();
            /// Macro generated public getter API.
            pub fn $lower() -> &'static $typ {
                $upper.get().unwrap()
            }
        )*

        struct BootInitCallBacks {
            $( $lower: fn(&'static Once<$typ>) -> (), )*
        }

        static BOOT_INIT_CALLBACKS: Once<BootInitCallBacks> = Once::new();

        /// The macro generated boot init callbacks registering interface.
        ///
        /// For the introduction of a new boot protocol, the entry point could be a novel
        /// one. The entry point function should register all the boot initialization
        /// methods before `ostd::main` is called. A boot initialization method takes a
        /// reference of the global static boot information variable and initialize it,
        /// so that the boot information it represents could be accessed in the kernel
        /// anywhere.
        ///
        /// The reason why the entry point function is not designed to directly initialize
        /// the boot information variables is simply that the heap is not initialized at
        /// that moment.
        pub fn register_boot_init_callbacks($( $lower: fn(&'static Once<$typ>) -> (), )* ) {
            BOOT_INIT_CALLBACKS.call_once(|| {
                BootInitCallBacks { $( $lower, )* }
            });
        }

        fn call_all_boot_init_callbacks() {
            let callbacks = &BOOT_INIT_CALLBACKS.get().unwrap();
            $( (callbacks.$lower)(&$upper); )*
        }
    };
}

// Define a series of static variables and their getter APIs.
define_global_static_boot_arguments!(
    //  Getter Names     |  Static Variables  | Variable Types
    bootloader_name,        BOOTLOADER_NAME,    String;
    kernel_cmdline,         KERNEL_CMDLINE,     KCmdlineArg;
    initramfs,              INITRAMFS,          &'static [u8];
    acpi_arg,               ACPI_ARG,           BootloaderAcpiArg;
    framebuffer_arg,        FRAMEBUFFER_ARG,    BootloaderFramebufferArg;
    memory_regions,         MEMORY_REGIONS,     Vec<MemoryRegion>;
);

/// The initialization method of the boot module.
///
/// After initializing the boot module, the get functions could be called.
/// The initialization must be done after the heap is set and before physical
/// mappings are cancelled.
pub fn init() {
    call_all_boot_init_callbacks();
}

/// Calls the OSTD-user defined entrypoint of the actual kernel.
///
/// Any kernel that uses the `ostd` crate should define a function marked with
/// `ostd::main` as the entrypoint.
pub fn call_ostd_main() -> ! {
    // Initialize the OSTD runtime.
    crate::init();

    // Set the global scheduler a FIFO scheduler to spawn the main function
    // as a kernel task.
    let simple_scheduler = Box::new(FifoScheduler::new());
    let static_scheduler: &'static dyn Scheduler = Box::leak(simple_scheduler);
    set_scheduler(static_scheduler);

    // Enable local IRQs in the kernel context.
    use crate::arch::irq;
    debug_assert!(
        !irq::is_local_enabled(),
        "IRQs should be disabled in the boot context"
    );
    irq::enable_local();

    let first_task = move || {
        #[cfg(not(ktest))]
        unsafe {
            // The entry point of kernel code, which should be defined by the package that
            // uses OSTD. The package should use the `ostd::main` macro to define it.
            extern "Rust" {
                fn __ostd_main() -> !;
            }
            __ostd_main();
        }
        #[cfg(ktest)]
        unsafe {
            // The whitelists that will be generated by OSDK runner as static consts.
            extern "Rust" {
                static KTEST_TEST_WHITELIST: Option<&'static [&'static str]>;
                static KTEST_CRATE_WHITELIST: Option<&'static [&'static str]>;
            }

            run_ktests(KTEST_TEST_WHITELIST, KTEST_CRATE_WHITELIST);
        }
    };

    let _ = TaskOptions::new(first_task).data(()).spawn();
    unreachable!("The spawned task will NOT return in the boot context");
}

fn run_ktests(test_whitelist: Option<&[&str]>, crate_whitelist: Option<&[&str]>) -> ! {
    use alloc::{boxed::Box, string::ToString};
    use core::any::Any;

    use crate::arch::qemu::{exit_qemu, QemuExitCode};

    let fn_catch_unwind = &(unwinding::panic::catch_unwind::<(), fn()>
        as fn(fn()) -> Result<(), Box<(dyn Any + Send + 'static)>>);

    use ostd_test::runner::{run_ktests, KtestResult};
    match run_ktests(
        &crate::console::early_print,
        fn_catch_unwind,
        test_whitelist.map(|s| s.iter().map(|s| s.to_string())),
        crate_whitelist,
    ) {
        KtestResult::Ok => exit_qemu(QemuExitCode::Success),
        KtestResult::Failed => exit_qemu(QemuExitCode::Failed),
    };
}
