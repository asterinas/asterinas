#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
// The no_mangle macro need to remove the `forbid(unsafe_code)` macro. The bootloader needs the _start function
// to be no mangle so that it can jump into the entry point.
// #![forbid(unsafe_code)]
#![test_runner(jinux_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
extern crate jinux_frame;

use core::panic::PanicInfo;
use jinux_frame::println;
#[cfg(feature = "intel_tdx")]
use jinux_frame::{config::PHYS_OFFSET, linux_boot::BootParams};

#[cfg(not(feature = "intel_tdx"))]
#[no_mangle]
pub fn jinux_main() -> ! {
    #[cfg(test)]
    test_main();
    jinux_frame::init();
    println!("[kernel] finish init jinux_frame");
    component::init_all(component::parse_metadata!()).unwrap();
    jinux_std::init();
    jinux_std::run_first_process();
}

#[cfg(feature = "intel_tdx")]
#[no_mangle]
pub fn jinux_main(boot_params: &'static BootParams) -> ! {
    #[cfg(test)]
    test_main();
    let rsdp_addr = boot_params.acpi_rsdp_addr;
    let memory = boot_params.e820_table;
    let _ramdisk_offset = boot_params.hdr.ramdisk_image;
    let _ramdisk_size = boot_params.hdr.ramdisk_size;
    jinux_frame::init(memory, rsdp_addr);
    println!("[kernel] finish init jinux_frame");
    component::init_all(component::parse_metadata!()).unwrap();
    jinux_std::init();
    jinux_std::run_first_process();
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    use jinux_frame::{exit_qemu, QemuExitCode};

    println!("[panic]:{:?}", info);
    jinux_frame::panic_handler();
    exit_qemu(QemuExitCode::Failed);
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    jinux_frame::test_panic_handler(info);
}

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}
