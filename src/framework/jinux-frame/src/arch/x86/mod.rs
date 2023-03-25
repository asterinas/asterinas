mod boot;
pub(crate) mod cpu;
pub mod device;
mod kernel;
pub(crate) mod mm;
pub(crate) mod timer;

use core::fmt::Write;
use alloc::fmt;
use log::{debug, info};
use trapframe::TrapFrame;
use x86_64::registers::{
    rflags::RFlags,
    segmentation::{Segment64, FS},
};

use crate::{
    trap::call_irq_callback_functions,
    user::{UserEvent, UserMode, UserModeExecute},
};

use self::cpu::CpuContext;

pub(crate) fn before_all_init() {
    enable_common_cpu_features();
    device::serial::init();
    boot::init();
}

pub(crate) fn after_all_init() {
    device::serial::callback_init();
    kernel::acpi::init();
    if kernel::xapic::has_apic() {
        kernel::ioapic::init();
        kernel::xapic::init();
    } else {
        info!("No apic exists, using pic instead");
        kernel::pic::enable();
    }
    timer::init();
    // Some driver like serial may use PIC
    kernel::pic::init();
}

pub(crate) fn interrupts_ack() {
    kernel::pic::ack();
    kernel::xapic::ack();
}

impl<'a> UserModeExecute for UserMode<'a> {
    fn execute(&mut self) -> crate::user::UserEvent {
        unsafe {
            self.user_space.vm_space().activate();
        }
        if !self.executed {
            self.context = self.user_space.cpu_ctx;
            if self.context.gp_regs.rflag == 0 {
                self.context.gp_regs.rflag = (RFlags::INTERRUPT_FLAG | RFlags::ID).bits() | 0x2;
            }
            // write fsbase
            unsafe {
                FS::write_base(x86_64::VirtAddr::new(self.user_space.cpu_ctx.fs_base));
            }
            let fp_regs = self.user_space.cpu_ctx.fp_regs;
            if fp_regs.is_valid() {
                fp_regs.restore();
            }
            self.executed = true;
        } else {
            // write fsbase
            if FS::read_base().as_u64() != self.context.fs_base {
                debug!("write fsbase: 0x{:x}", self.context.fs_base);
                unsafe {
                    FS::write_base(x86_64::VirtAddr::new(self.context.fs_base));
                }
            }
        }
        self.user_context = self.context.into();
        self.user_context.run();
        let mut trap_frame;
        while self.user_context.trap_num >= 0x20 && self.user_context.trap_num < 0x100 {
            trap_frame = TrapFrame {
                rax: self.user_context.general.rax,
                rbx: self.user_context.general.rbx,
                rcx: self.user_context.general.rcx,
                rdx: self.user_context.general.rdx,
                rsi: self.user_context.general.rsi,
                rdi: self.user_context.general.rdi,
                rbp: self.user_context.general.rbp,
                rsp: self.user_context.general.rsp,
                r8: self.user_context.general.r8,
                r9: self.user_context.general.r9,
                r10: self.user_context.general.r10,
                r11: self.user_context.general.r11,
                r12: self.user_context.general.r12,
                r13: self.user_context.general.r13,
                r14: self.user_context.general.r14,
                r15: self.user_context.general.r15,
                _pad: 0,
                trap_num: self.user_context.trap_num,
                error_code: self.user_context.error_code,
                rip: self.user_context.general.rip,
                cs: 0,
                rflags: self.user_context.general.rflags,
            };
            call_irq_callback_functions(&mut trap_frame);
            self.user_context.run();
        }
        x86_64::instructions::interrupts::enable();
        self.context = CpuContext::from(self.user_context);
        self.context.fs_base = FS::read_base().as_u64();
        if self.user_context.trap_num != 0x100 {
            UserEvent::Exception
        } else {
            UserEvent::Syscall
        }
    }
}

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes() {
            device::serial::send(c);
        }
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! print {
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::arch::x86::print(format_args!($fmt $(, $($arg)+)?))
  }
}

#[macro_export]
macro_rules! println {
  ($fmt: literal $(, $($arg: tt)+)?) => {
    $crate::arch::x86::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
  }
}

fn enable_common_cpu_features() {
    use x86_64::registers::{control::Cr4Flags, model_specific::EferFlags, xcontrol::XCr0Flags};
    let mut cr4 = x86_64::registers::control::Cr4::read();
    cr4 |= Cr4Flags::FSGSBASE | Cr4Flags::OSXSAVE | Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE;
    unsafe {
        x86_64::registers::control::Cr4::write(cr4);
    }

    let mut xcr0 = x86_64::registers::xcontrol::XCr0::read();
    xcr0 |= XCr0Flags::AVX | XCr0Flags::SSE;
    unsafe {
        x86_64::registers::xcontrol::XCr0::write(xcr0);
    }

    unsafe {
        // enable non-executable page protection
        x86_64::registers::model_specific::Efer::update(|efer| {
            *efer |= EferFlags::NO_EXECUTE_ENABLE;
        });
    }
}
