// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

use ostd::{
    arch::vm::{GuestContext, VmxExitReason},
    mm::{CachePolicy, FrameAllocOptions, PAGE_SIZE, PageFlags, PageProperty, VmIo},
    power::{ExitCode, poweroff},
    prelude::*,
    task::{TaskOptions, disable_preempt},
    vm::{GuestInterruptPort, GuestMode, GuestPhysMemSpace, GuestRunResult, GuestTimerPort},
};

#[ostd::main]
fn main() {
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

    // Start the real-mode guest at GPA 0 instead of the x86 reset vector.
    let mut context = GuestContext::new(0).unwrap();
    let mut regs = context.regs();
    regs.rip = 0;
    context.set_regs(regs);
    let mut sregs = context.sregs();
    sregs.cs.selector = 0;
    sregs.cs.base = 0;
    context.set_sregs(sregs);

    let no_events = NoGuestEvents;
    loop {
        let exit = match guest_mode
            .execute(&mut context, &guest_mem, &no_events, &no_events)
            .expect("guest execution failed")
        {
            GuestRunResult::VmExit(exit) => exit,
            GuestRunResult::HostInterrupt => continue,
            GuestRunResult::WaitForSipi => panic!("unexpected startup wait for the bootstrap vCPU"),
        };

        match exit.exit_reason {
            reason
                if reason == VmxExitReason::IO_INSTRUCTION as u32
                    && exit.exit_qualification & 0x3f == 0
                    && (exit.exit_qualification >> 16) as u16 == 0x3f8 =>
            {
                print!("{}", char::from(context.regs().rax as u8));
                context.advance_rip(u64::from(exit.instruction_len));
            }
            reason if reason == VmxExitReason::HLT as u32 => break,
            reason => panic!("unsupported guest exit: {reason:#x}"),
        }
    }
    // Drop the context and guest memory before releasing VMX and powering off.
}

// Machine code for hello.S, a 16-bit real-mode guest.
// Each MOV AL, imm8 / OUT DX, AL pair outputs one byte to COM1.
const GUEST_CODE: &[u8] = &[
    0xba, 0xf8, 0x03, // mov dx, 0x3f8
    0xb0, b'H', 0xee, // H
    0xb0, b'e', 0xee, // e
    0xb0, b'l', 0xee, // l
    0xb0, b'l', 0xee, // l
    0xb0, b'o', 0xee, // o
    0xb0, b' ', 0xee, // space
    0xb0, b'W', 0xee, // W
    0xb0, b'o', 0xee, // o
    0xb0, b'r', 0xee, // r
    0xb0, b'l', 0xee, // l
    0xb0, b'd', 0xee, // d
    0xb0, b'\n', 0xee, // newline
    0xf4, // hlt
];

// Satisfy the execution interface without offering guest interrupts or timers.
struct NoGuestEvents;
impl GuestInterruptPort for NoGuestEvents {}
impl GuestTimerPort for NoGuestEvents {}
