use core::fmt;
use core::ops;
use gimli::{AArch64, Register};

use super::maybe_cfi;

// Match DWARF_FRAME_REGISTERS in libgcc
pub const MAX_REG_RULES: usize = 97;

#[repr(C)]
#[derive(Clone, Default)]
pub struct Context {
    pub gp: [usize; 31],
    pub sp: usize,
    pub fp: [usize; 32],
}

impl fmt::Debug for Context {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fmt = fmt.debug_struct("Context");
        for i in 0..=30 {
            fmt.field(
                AArch64::register_name(Register(i as _)).unwrap(),
                &self.gp[i],
            );
        }
        fmt.field("sp", &self.sp);
        for i in 0..=31 {
            fmt.field(
                AArch64::register_name(Register((i + 64) as _)).unwrap(),
                &self.fp[i],
            );
        }
        fmt.finish()
    }
}

impl ops::Index<Register> for Context {
    type Output = usize;

    fn index(&self, reg: Register) -> &usize {
        match reg {
            Register(0..=30) => &self.gp[reg.0 as usize],
            AArch64::SP => &self.sp,
            Register(64..=95) => &self.fp[(reg.0 - 64) as usize],
            _ => unimplemented!(),
        }
    }
}

impl ops::IndexMut<gimli::Register> for Context {
    fn index_mut(&mut self, reg: Register) -> &mut usize {
        match reg {
            Register(0..=30) => &mut self.gp[reg.0 as usize],
            AArch64::SP => &mut self.sp,
            Register(64..=95) => &mut self.fp[(reg.0 - 64) as usize],
            _ => unimplemented!(),
        }
    }
}

macro_rules! save {
    (gp$(, $fp:ident)?) => {
        // No need to save caller-saved registers here.
        core::arch::naked_asm!(
            maybe_cfi!(".cfi_startproc"),
            "stp x29, x30, [sp, -16]!",
            maybe_cfi!("
            .cfi_def_cfa_offset 16
            .cfi_offset x29, -16
            .cfi_offset x30, -8
            "),
            "sub sp, sp, 512",
            maybe_cfi!(".cfi_def_cfa_offset 528"),
            "
            mov x8, x0
            mov x0, sp
            ",
            save!(maybesavefp($($fp)?)),
            "
            str x19, [sp, 0x98]
            stp x20, x21, [sp, 0xA0]
            stp x22, x23, [sp, 0xB0]
            stp x24, x25, [sp, 0xC0]
            stp x26, x27, [sp, 0xD0]
            stp x28, x29, [sp, 0xE0]
            add x2, sp, 528
            stp x30, x2, [sp, 0xF0]

            blr x8

            add sp, sp, 512
            ",
            maybe_cfi!(".cfi_def_cfa_offset 16"),
            "ldp x29, x30, [sp], 16",
            maybe_cfi!("
            .cfi_def_cfa_offset 0
            .cfi_restore x29
            .cfi_restore x30
            "),
            "ret",
            maybe_cfi!(".cfi_endproc"),
        );
    };
    (maybesavefp(fp)) => {
        "
        stp d8, d9, [sp, 0x140]
        stp d10, d11, [sp, 0x150]
        stp d12, d13, [sp, 0x160]
        stp d14, d15, [sp, 0x170]
        "
    };
    (maybesavefp()) => { "" };
}

#[unsafe(naked)]
pub extern "C-unwind" fn save_context(f: extern "C" fn(&mut Context, *mut ()), ptr: *mut ()) {
    #[cfg(target_feature = "neon")]
    save!(gp, fp);
    #[cfg(not(target_feature = "neon"))]
    save!(gp);
}

macro_rules! restore {
    ($ctx:expr, gp$(, $fp:ident)?) => {
        core::arch::asm!(
            restore!(mayberestore($($fp)?)),
            "
            ldp x2, x3, [x0, 0x10]
            ldp x4, x5, [x0, 0x20]
            ldp x6, x7, [x0, 0x30]
            ldp x8, x9, [x0, 0x40]
            ldp x10, x11, [x0, 0x50]
            ldp x12, x13, [x0, 0x60]
            ldp x14, x15, [x0, 0x70]
            ldp x16, x17, [x0, 0x80]
            ldp x18, x19, [x0, 0x90]
            ldp x20, x21, [x0, 0xA0]
            ldp x22, x23, [x0, 0xB0]
            ldp x24, x25, [x0, 0xC0]
            ldp x26, x27, [x0, 0xD0]
            ldp x28, x29, [x0, 0xE0]
            ldp x30, x1, [x0, 0xF0]
            mov sp, x1

            ldp x0, x1, [x0, 0x00]
            ret
            ",
            in("x0") $ctx,
            options(noreturn)
        );
    };
    (mayberestore(fp)) => {
        "
        ldp d0, d1, [x0, 0x100]
        ldp d2, d3, [x0, 0x110]
        ldp d4, d5, [x0, 0x120]
        ldp d6, d7, [x0, 0x130]
        ldp d8, d9, [x0, 0x140]
        ldp d10, d11, [x0, 0x150]
        ldp d12, d13, [x0, 0x160]
        ldp d14, d15, [x0, 0x170]
        ldp d16, d17, [x0, 0x180]
        ldp d18, d19, [x0, 0x190]
        ldp d20, d21, [x0, 0x1A0]
        ldp d22, d23, [x0, 0x1B0]
        ldp d24, d25, [x0, 0x1C0]
        ldp d26, d27, [x0, 0x1D0]
        ldp d28, d29, [x0, 0x1E0]
        ldp d30, d31, [x0, 0x1F0]
        "
    };
    (mayberestore()) => { "" };
}

pub unsafe fn restore_context(ctx: &Context) -> ! {
    unsafe {
        #[cfg(target_feature = "neon")]
        restore!(ctx, gp, fp);
        #[cfg(not(target_feature = "neon"))]
        restore!(ctx, gp);
    }
}
