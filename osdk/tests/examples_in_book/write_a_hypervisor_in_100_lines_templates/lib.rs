// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

use ostd::{
    arch::vm::{GuestContext, VcpuMsrs, VcpuRegs, VcpuSregs, VmxExitReason},
    mm::{CachePolicy, FrameAllocOptions, PAGE_SIZE, PageFlags, PageProperty, VmIo},
    power::{ExitCode, poweroff},
    prelude::*,
    task::{TaskOptions, disable_preempt},
    vm::{GuestInterruptPort, GuestMode, GuestPhysMemSpace, GuestRunResult, GuestTimerPort},
};

#[ostd::main]
fn main() {
    ostd::log::set_max_level(ostd::log::LevelFilter::Info);

    // VMX lifecycle operations need a task context with interrupts enabled.
    TaskOptions::new(|| {
        run_guest();
        println!("Guest halted; returned to host.");
        poweroff(ExitCode::Success);
    })
    .spawn()
    .unwrap();
}

fn run_guest() {
    let guest_mode = GuestMode::new().expect("VMX is required to run the guest");
    let guest_mem = GuestPhysMemSpace::new().expect("EPT is required for guest memory");
    let frame = FrameAllocOptions::new().alloc_frame().unwrap();
    frame.write_bytes(0, GUEST_CODE).unwrap();
    {
        let guard = disable_preempt();
        let mut cursor = guest_mem.cursor_mut(&guard, &(0..PAGE_SIZE)).unwrap();
        cursor.map(
            frame.into(),
            PageProperty::new_user(PageFlags::RWX, CachePolicy::Writeback),
        );
    }

    let mut context =
        GuestContext::new(VcpuRegs::new(), VcpuSregs::new(), VcpuMsrs::default()).unwrap();

    let no_events = NoGuestEvents;
    loop {
        let exit = match guest_mode
            .execute(&mut context, &guest_mem, &no_events, &no_events)
            .unwrap()
        {
            GuestRunResult::VmExit(exit) => exit,
            GuestRunResult::HostInterrupt => continue,
        };

        match exit.exit_reason {
            reason
                if reason == VmxExitReason::IO_INSTRUCTION as u32
                    && exit.exit_qualification & 0x3f == 0
                    && (exit.exit_qualification >> 16) as u16 == 0x3f8 =>
            {
                print!("{}", char::from(context.regs().rax as u8));
                let regs = context.regs_mut();
                regs.rip = regs.rip.checked_add(exit.instruction_len as usize).unwrap();
            }
            reason if reason == VmxExitReason::HLT as u32 => break,
            reason => panic!("unsupported guest exit: {reason:#x}"),
        }
    }
    // Drop the context and guest memory before releasing VMX and powering off.
}

// The Makefile assembles and links guest_hello.S as a flat binary at GPA 0.
const GUEST_CODE: &[u8] = include_bytes!("../guest_hello");

// Satisfy the execution interface without offering guest interrupts or timers.
struct NoGuestEvents;
impl GuestInterruptPort for NoGuestEvents {}
impl GuestTimerPort for NoGuestEvents {}
