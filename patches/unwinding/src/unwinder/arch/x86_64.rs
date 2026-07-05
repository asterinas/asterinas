use core::fmt;
use core::ops;
use gimli::{Register, X86_64};

use super::maybe_cfi;

// Match DWARF_FRAME_REGISTERS in libgcc
pub const MAX_REG_RULES: usize = 17;

#[repr(C)]
#[derive(Clone, Default)]
pub struct Context {
    pub registers: [usize; 16],
    pub ra: usize,
    pub mcxsr: usize,
    pub fcw: usize,
}

impl fmt::Debug for Context {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fmt = fmt.debug_struct("Context");
        for i in 0..=15 {
            fmt.field(
                X86_64::register_name(Register(i as _)).unwrap(),
                &self.registers[i],
            );
        }
        fmt.field("ra", &self.ra)
            .field("mcxsr", &self.mcxsr)
            .field("fcw", &self.fcw)
            .finish()
    }
}

impl ops::Index<Register> for Context {
    type Output = usize;

    fn index(&self, reg: Register) -> &usize {
        match reg {
            Register(0..=15) => &self.registers[reg.0 as usize],
            X86_64::RA => &self.ra,
            X86_64::MXCSR => &self.mcxsr,
            X86_64::FCW => &self.fcw,
            _ => unimplemented!(),
        }
    }
}

impl ops::IndexMut<gimli::Register> for Context {
    fn index_mut(&mut self, reg: Register) -> &mut usize {
        match reg {
            Register(0..=15) => &mut self.registers[reg.0 as usize],
            X86_64::RA => &mut self.ra,
            X86_64::MXCSR => &mut self.mcxsr,
            X86_64::FCW => &mut self.fcw,
            _ => unimplemented!(),
        }
    }
}

#[unsafe(naked)]
pub extern "C-unwind" fn save_context(f: extern "C" fn(&mut Context, *mut ()), ptr: *mut ()) {
    // No need to save caller-saved registers here.
    core::arch::naked_asm!(
        maybe_cfi!(".cfi_startproc"),
        "sub rsp, 0x98",
        maybe_cfi!(".cfi_def_cfa_offset 0xA0"),
        "
            mov [rsp + 0x18], rbx
            mov [rsp + 0x30], rbp

            /* Adjust the stack to account for the return address */
            lea rax, [rsp + 0xA0]
            mov [rsp + 0x38], rax

            mov [rsp + 0x60], r12
            mov [rsp + 0x68], r13
            mov [rsp + 0x70], r14
            mov [rsp + 0x78], r15

            /* Return address */
            mov rax, [rsp + 0x98]
            mov [rsp + 0x80], rax

            stmxcsr [rsp + 0x88]
            fnstcw [rsp + 0x90]

            mov rax, rdi
            mov rdi, rsp
            call rax
            add rsp, 0x98
            ",
        maybe_cfi!(".cfi_def_cfa_offset 8"),
        "ret",
        maybe_cfi!(".cfi_endproc"),
    );
}

pub unsafe fn restore_context(ctx: &Context) -> ! {
    unsafe {
        core::arch::asm!(
            "
            /* Restore stack */
            mov rsp, [rdi + 0x38]

            /* Restore callee-saved control registers */
            ldmxcsr [rdi + 0x88]
            fldcw [rdi + 0x90]

            /* Restore return address */
            mov rax, [rdi + 0x80]
            push rax

            /*
            * Restore general-purpose registers. Non-callee-saved registers are
            * also restored because sometimes it's used to pass unwind arguments.
            */
            mov rax, [rdi + 0x00]
            mov rdx, [rdi + 0x08]
            mov rcx, [rdi + 0x10]
            mov rbx, [rdi + 0x18]
            mov rsi, [rdi + 0x20]
            mov rbp, [rdi + 0x30]
            mov r8 , [rdi + 0x40]
            mov r9 , [rdi + 0x48]
            mov r10, [rdi + 0x50]
            mov r11, [rdi + 0x58]
            mov r12, [rdi + 0x60]
            mov r13, [rdi + 0x68]
            mov r14, [rdi + 0x70]
            mov r15, [rdi + 0x78]

            /* RDI restored last */
            mov rdi, [rdi + 0x28]

            ret
            ",
            in("rdi") ctx,
            options(noreturn)
        );
    }
}
