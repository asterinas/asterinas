// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

use ostd::{
    arch::vm::{self as arch_vm, GuestContext, GuestException},
    mm::{CachePolicy, FrameAllocOptions, PAGE_SIZE, PageFlags, PageProperty, VmIo},
    power::{ExitCode, poweroff},
    prelude::*,
    task::TaskOptions,
    vm::{DummyGuestHooks, GuestMode, GuestPhysMemSpace, GuestReturnReason},
};

/// The guest physical address at which the guest sees the serial device.
const GUEST_UART_ADDR: usize = 0x1000;
/// The host physical address of the UART0 device on the QEMU `virt` machine.
const HOST_UART_ADDR: usize = 0x1000_0000;

#[ostd::main]
fn main() {
    // VMX lifecycle operations need a task context with interrupts enabled.
    TaskOptions::new(|| {
        run_guest();
        poweroff(ExitCode::Success);
    })
    .spawn()
    .unwrap();
}

fn run_guest() {
    println!("KVM test start.");
    let guest_mode = match GuestMode::new() {
        Ok(guest_mode) => guest_mode,
        Err(err) => panic!("Failed to initialize guest_mode: {err:?}"),
    };

    // Prepares the guest code by writing to the guest memory space.
    let guest_mem = GuestPhysMemSpace::new().expect("EPT is required for guest memory");
    let frame = FrameAllocOptions::new().alloc_frame().unwrap();
    frame.write_bytes(0, GUEST_CODE).unwrap();
    guest_mem
        .map(
            0,
            frame.paddr(),
            PAGE_SIZE,
            PageProperty::new_user(PageFlags::RWX, CachePolicy::Writeback),
        )
        .unwrap();
    // Maps the host UART into the guest physical address space so that the
    // guest's serial output reaches QEMU's console directly.
    guest_mem
        .map(
            GUEST_UART_ADDR,
            HOST_UART_ADDR,
            PAGE_SIZE,
            PageProperty::new_user(PageFlags::RWX, CachePolicy::Uncacheable),
        )
        .unwrap();

    let mut context = GuestContext::new();

    loop {
        let exit = match guest_mode
            .execute(&mut context, &guest_mem, &DummyGuestHooks)
            .unwrap()
        {
            GuestReturnReason::VmExit(exit) => exit,
            GuestReturnReason::KernelEvent => continue,
        };

        debug_assert!(exit.scause & arch_vm::INTERRUPT_BIT == 0);
        let code = exit.scause & !arch_vm::INTERRUPT_BIT;
        match GuestException::try_from(code) {
            Ok(GuestException::VIRTUAL_INSTRUCTION) => {
                const WFI: usize = 0x1050_0073;

                assert_eq!(
                    exit.stval, WFI,
                    "unexpected virtual instruction: {:#x}",
                    exit.stval
                );

                println!("Guest halted; returned to host.");
                break;
            }
            Ok(scause_code) => panic!("unsupported guest exit: {scause_code:?}"),
            Err(_) => panic!("unknown guest exception {code}"),
        }
    }
}

// The Makefile assembles and links `guest.S` as a flat binary at GPA 0.
const GUEST_CODE: &[u8] = include_bytes!("../guest");
