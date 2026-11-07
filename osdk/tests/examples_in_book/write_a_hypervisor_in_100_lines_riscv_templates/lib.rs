// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

use ostd::{
    arch::vm::{GuestContext, RiscvGuestHooks, VcpuArchState},
    mm::{CachePolicy, FrameAllocOptions, PAGE_SIZE, PageFlags, PageProperty, VmIo},
    power::{ExitCode, poweroff},
    prelude::*,
    task::TaskOptions,
    vm::{GuestMode, GuestPhysMemSpace, GuestReturnReason},
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
            PageProperty::new_guest(PageFlags::RWX, CachePolicy::Writeback),
        )
        .unwrap();
    // Maps the host UART into the guest physical address space so that the
    // guest's serial output reaches QEMU's console directly.
    guest_mem
        .map(
            GUEST_UART_ADDR,
            HOST_UART_ADDR,
            PAGE_SIZE,
            PageProperty::new_guest(PageFlags::RWX, CachePolicy::Uncacheable),
        )
        .unwrap();

    let mut state = VcpuArchState::default();
    state.init();
    state.context_reset();
    state.fp_reset();
    // The guest code is mapped at guest physical address 0, which is also the
    // address the guest starts executing from.
    state.guest_context.sepc = 0;

    state.config_ran_once();

    // Creates the data structure that maintains the CPU state of the guest.
    let mut context = GuestContext::new(state);

    ostd::arch::vm::enable_virtualization_cpu();

    state.vcpu_load();

    loop {
        // Runs the guest until an exit requires handling by the host.
        match guest_mode
            .execute(&mut context, &guest_mem, &RiscvGuestHooks)
            .unwrap()
        {
            // The guest executes `wfi` after printing. As `hstatus.VTW` is
            // set, that instruction traps and ends the guest here.
            GuestReturnReason::VmExit(_) => break,
            GuestReturnReason::KernelEvent => continue,
        }
    }

    context.arch().vcpu_put();

    ostd::arch::vm::disable_virtualization_cpu();

    println!("Guest halted; returned to host.");
}

// The Makefile assembles and links `guest.S` as a flat binary at GPA 0.
const GUEST_CODE: &[u8] = include_bytes!("../guest");
