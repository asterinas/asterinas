//! The architecture-independent boot module, which provides a universal interface
//! from the bootloader to the rest of the framework.
//!

pub mod kcmdline;
pub mod memory_region;

use kcmdline::KCmdlineArg;

use self::memory_region::MemoryRegion;

use alloc::{string::String, vec::Vec};
use spin::Once;

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
    pub address: usize,
    pub width: usize,
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
        /// methods before `aster_main` is called. A boot initialization method takes a
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

/// Call the framework-user defined entrypoint of the actual kernel.
///
/// Any kernel that uses the aster-frame crate should define a function named
/// `aster_main` as the entrypoint.
pub fn call_aster_main() -> ! {
    #[cfg(not(ktest))]
    unsafe {
        // The entry point of kernel code, which should be defined by the package that
        // uses aster-frame.
        extern "Rust" {
            fn aster_main() -> !;
        }
        aster_main();
    }
    #[cfg(ktest)]
    {
        use crate::arch::qemu::{exit_qemu, QemuExitCode};
        use alloc::{boxed::Box, string::ToString};
        use core::any::Any;
        crate::init();
        let fn_catch_unwind = &(unwinding::panic::catch_unwind::<(), fn()>
            as fn(fn()) -> Result<(), Box<(dyn Any + Send + 'static)>>);
        // Parse the whitelist from the kernel command line.
        let mut paths = None;
        let args = kernel_cmdline().get_module_args("ktest");
        if let Some(args) = args {
            for options in args {
                match options {
                    kcmdline::ModuleArg::KeyVal(key, val) => {
                        if key.to_str().unwrap() == "whitelist" && val.to_str().unwrap() != "" {
                            let paths_str = val.to_str().unwrap();
                            paths = Some(
                                paths_str
                                    .split(',')
                                    .map(|s| s.to_string())
                                    .collect::<Vec<_>>(),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        use ktest::runner::{run_ktests, KtestResult};
        match run_ktests(
            &crate::console::print,
            fn_catch_unwind,
            paths.map(|v| v.into_iter()),
        ) {
            KtestResult::Ok => exit_qemu(QemuExitCode::Success),
            KtestResult::Failed => exit_qemu(QemuExitCode::Failed),
        }
    }
}
