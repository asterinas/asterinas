//! The Jinux test module providing a individual crate testing framework

use core::panic::PanicInfo;
use crate::{print, println};

pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        print!("{}...\n", core::any::type_name::<T>());
        self();
        println!("[ok]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("[panic]:{:?}", info);
    /*let mut fp: usize;
    let stop = unsafe {
        crate::task::Task::current().kstack.get_top()
    };
    log::info!("stop:{:x}",stop);
    unsafe{
        asm!("mov rbp, {}", out(reg) fp);
        log::info!("fp:{:x}",fp);
        println!("---START BACKTRACE---");
        for i in 0..10 {
        if fp == stop {
                break;
            }
            println!("#{}:ra={:#x}", i, *((fp - 8) as *const usize));
            log::info!("fp target:{:x}",*((fp ) as *const usize));
            fp = *((fp - 16) as *const usize);
        }
        println!("---END   BACKTRACE---");
    }*/
    exit_qemu(QemuExitCode::Failed);
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("[failed]");
    println!("Error: {}", info);
    exit_qemu(QemuExitCode::Failed);
}

/// The exit code of x86 QEMU isa debug device. In `qemu-system-x86_64` the
/// exit code will be `(code << 1) | 1`. So you could never let QEMU invoke
/// `exit(0)`. We also need to check if the exit code is returned by the
/// kernel, so we couldn't use 0 as exit_success because this may conflict
/// with QEMU return value 1, which indicates that QEMU itself fails.
#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x20,
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;
    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
    unreachable!()
}

#[cfg(feature = "coverage")]
pub fn get_llvm_coverage_raw() -> Vec<u8> {
    let mut coverage = vec![];
    // Safety: minicov::capture_coverage is not thread safe.
    // There mustn't be any races here.
    unsafe {
        minicov::capture_coverage(&mut coverage).unwrap();
    }
    minicov::reset_coverage();
    coverage
}
