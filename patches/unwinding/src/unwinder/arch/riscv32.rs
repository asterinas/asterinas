use core::fmt;
use core::ops;
use gimli::{Register, RiscV};

use super::maybe_cfi;

// Match DWARF_FRAME_REGISTERS in libgcc
pub const MAX_REG_RULES: usize = 65;

#[cfg(all(target_feature = "e", target_feature = "f"))]
compile_error!("RISC-V RV32E with F extension is not supported");

#[repr(C)]
#[derive(Clone, Default)]
pub struct Context {
    #[cfg(not(target_feature = "e"))]
    pub gp: [usize; 32],
    #[cfg(target_feature = "e")]
    pub gp: [usize; 16],
    #[cfg(all(target_feature = "f", not(target_feature = "d")))]
    pub fp: [u32; 32],
    #[cfg(target_feature = "d")]
    pub fp: [u64; 32],
}

impl fmt::Debug for Context {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fmt = fmt.debug_struct("Context");
        for (i, gp) in self.gp.iter().enumerate() {
            fmt.field(RiscV::register_name(Register(i as _)).unwrap(), gp);
        }
        #[cfg(target_feature = "f")]
        for i in 0..=31 {
            fmt.field(
                RiscV::register_name(Register((i + 32) as _)).unwrap(),
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
            Register(0..=31) => &self.gp[reg.0 as usize],
            // We cannot support indexing fp here. It is 64-bit if D extension is implemented,
            // and 32-bit if only F extension is implemented.
            _ => unimplemented!(),
        }
    }
}

impl ops::IndexMut<gimli::Register> for Context {
    fn index_mut(&mut self, reg: Register) -> &mut usize {
        match reg {
            Register(0..=31) => &mut self.gp[reg.0 as usize],
            // We cannot support indexing fp here. It is 64-bit if D extension is implemented,
            // and 32-bit if only F extension is implemented.
            _ => unimplemented!(),
        }
    }
}

macro_rules! code {
    // RV32*
    (save_gp_0_to_15) => {
        "
        sw x0, 0x00(sp)
        sw ra, 0x04(sp)
        sw t0, 0x08(sp)
        sw gp, 0x0C(sp)
        sw tp, 0x10(sp)
        sw s0, 0x20(sp)
        sw s1, 0x24(sp)
        "
    };
    // RV32I*
    (save_gp_16_to_31) => {
        "
        sw s2, 0x48(sp)
        sw s3, 0x4C(sp)
        sw s4, 0x50(sp)
        sw s5, 0x54(sp)
        sw s6, 0x58(sp)
        sw s7, 0x5C(sp)
        sw s8, 0x60(sp)
        sw s9, 0x64(sp)
        sw s10, 0x68(sp)
        sw s11, 0x6C(sp)
        "
    };
    // RV32IF
    (save_fp_f) => {
        // arch option manipulation needed due to LLVM/Rust bug, see rust-lang/rust#80608
        "
        .option push
        .option arch, +f
        fsw fs0, 0xA0(sp)
        fsw fs1, 0xA4(sp)
        fsw fs2, 0xC8(sp)
        fsw fs3, 0xCC(sp)
        fsw fs4, 0xD0(sp)
        fsw fs5, 0xD4(sp)
        fsw fs6, 0xD8(sp)
        fsw fs7, 0xDC(sp)
        fsw fs8, 0xE0(sp)
        fsw fs9, 0xE4(sp)
        fsw fs10, 0xE8(sp)
        fsw fs11, 0xEC(sp)
        .option pop
        "
    };
    // RV32IFD
    (save_fp_d) => {
        // arch option manipulation needed due to LLVM/Rust bug, see rust-lang/rust#80608
        "
        .option push
        .option arch, +d
        fsd fs0, 0xC0(sp)
        fsd fs1, 0xC8(sp)
        fsd fs2, 0x110(sp)
        fsd fs3, 0x118(sp)
        fsd fs4, 0x120(sp)
        fsd fs5, 0x128(sp)
        fsd fs6, 0x130(sp)
        fsd fs7, 0x138(sp)
        fsd fs8, 0x140(sp)
        fsd fs9, 0x148(sp)
        fsd fs10, 0x150(sp)
        fsd fs11, 0x158(sp)
        .option pop
        "
    };
    // RV32*
    (restore_gp_0_to_15) => {
        "
        lw ra, 0x04(a0)
        lw sp, 0x08(a0)
        lw gp, 0x0C(a0)
        lw tp, 0x10(a0)
        lw t0, 0x14(a0)
        lw t1, 0x18(a0)
        lw t2, 0x1C(a0)
        lw s0, 0x20(a0)
        lw s1, 0x24(a0)
        lw a1, 0x2C(a0)
        lw a2, 0x30(a0)
        lw a3, 0x34(a0)
        lw a4, 0x38(a0)
        lw a5, 0x3C(a0)
        "
    };
    // RV32I*
    (restore_gp_16_to_31) => {
        "
        lw a6, 0x40(a0)
        lw a7, 0x44(a0)
        lw s2, 0x48(a0)
        lw s3, 0x4C(a0)
        lw s4, 0x50(a0)
        lw s5, 0x54(a0)
        lw s6, 0x58(a0)
        lw s7, 0x5C(a0)
        lw s8, 0x60(a0)
        lw s9, 0x64(a0)
        lw s10, 0x68(a0)
        lw s11, 0x6C(a0)
        lw t3, 0x70(a0)
        lw t4, 0x74(a0)
        lw t5, 0x78(a0)
        lw t6, 0x7C(a0)
        "
    };
    // RV32IF
    (restore_fp_f) => {
        "
        flw ft0, 0x80(a0)
        flw ft1, 0x84(a0)
        flw ft2, 0x88(a0)
        flw ft3, 0x8C(a0)
        flw ft4, 0x90(a0)
        flw ft5, 0x94(a0)
        flw ft6, 0x98(a0)
        flw ft7, 0x9C(a0)
        flw fs0, 0xA0(a0)
        flw fs1, 0xA4(a0)
        flw fa0, 0xA8(a0)
        flw fa1, 0xAC(a0)
        flw fa2, 0xB0(a0)
        flw fa3, 0xB4(a0)
        flw fa4, 0xB8(a0)
        flw fa5, 0xBC(a0)
        flw fa6, 0xC0(a0)
        flw fa7, 0xC4(a0)
        flw fs2, 0xC8(a0)
        flw fs3, 0xCC(a0)
        flw fs4, 0xD0(a0)
        flw fs5, 0xD4(a0)
        flw fs6, 0xD8(a0)
        flw fs7, 0xDC(a0)
        flw fs8, 0xE0(a0)
        flw fs9, 0xE4(a0)
        flw fs10, 0xE8(a0)
        flw fs11, 0xEC(a0)
        flw ft8, 0xF0(a0)
        flw ft9, 0xF4(a0)
        flw ft10, 0xF8(a0)
        flw ft11, 0xFC(a0)
        "
    };
    // RV32IFD
    (restore_fp_d) => {
        "
        fld ft0, 0x80(a0)
        fld ft1, 0x88(a0)
        fld ft2, 0x90(a0)
        fld ft3, 0x98(a0)
        fld ft4, 0xA0(a0)
        fld ft5, 0xA8(a0)
        fld ft6, 0xB0(a0)
        fld ft7, 0xB8(a0)
        fld fs0, 0xC0(a0)
        fld fs1, 0xC8(a0)
        fld fa0, 0xD0(a0)
        fld fa1, 0xD8(a0)
        fld fa2, 0xE0(a0)
        fld fa3, 0xE8(a0)
        fld fa4, 0xF0(a0)
        fld fa5, 0xF8(a0)
        fld fa6, 0x100(a0)
        fld fa7, 0x108(a0)
        fld fs2, 0x110(a0)
        fld fs3, 0x118(a0)
        fld fs4, 0x120(a0)
        fld fs5, 0x128(a0)
        fld fs6, 0x130(a0)
        fld fs7, 0x138(a0)
        fld fs8, 0x140(a0)
        fld fs9, 0x148(a0)
        fld fs10, 0x150(a0)
        fld fs11, 0x158(a0)
        fld ft8, 0x160(a0)
        fld ft9, 0x168(a0)
        fld ft10, 0x170(a0)
        fld ft11, 0x178(a0)
        "
    };
}

#[unsafe(naked)]
pub extern "C-unwind" fn save_context(f: extern "C" fn(&mut Context, *mut ()), ptr: *mut ()) {
    // No need to save caller-saved registers here.
    // RV32IFD
    #[cfg(target_feature = "d")]
    core::arch::naked_asm!(
        maybe_cfi!(".cfi_startproc"),
        "
        mv t0, sp
        add sp, sp, -0x190
        ",
        maybe_cfi!(".cfi_def_cfa_offset 0x190"),
        "sw ra, 0x180(sp)",
        maybe_cfi!(".cfi_offset ra, -16"),
        code!(save_gp_0_to_15),
        code!(save_gp_16_to_31),
        code!(save_fp_d),
        "
        mv t0, a0
        mv a0, sp
        jalr t0
        lw ra, 0x180(sp)
        add sp, sp, 0x190
        ",
        maybe_cfi!(".cfi_def_cfa_offset 0"),
        maybe_cfi!(".cfi_restore ra"),
        "ret",
        maybe_cfi!(".cfi_endproc"),
    );
    // RV32IF
    #[cfg(all(target_feature = "f", not(target_feature = "d")))]
    core::arch::naked_asm!(
        maybe_cfi!(".cfi_startproc"),
        "
        mv t0, sp
        add sp, sp, -0x110
        ",
        maybe_cfi!(".cfi_def_cfa_offset 0x110"),
        "sw ra, 0x100(sp)",
        maybe_cfi!(".cfi_offset ra, -16"),
        code!(save_gp_0_to_15),
        code!(save_gp_16_to_31),
        code!(save_fp_f),
        "
        mv t0, a0
        mv a0, sp
        jalr t0
        lw ra, 0x100(sp)
        add sp, sp, 0x110
        ",
        maybe_cfi!(".cfi_def_cfa_offset 0"),
        maybe_cfi!(".cfi_restore ra"),
        "ret",
        maybe_cfi!(".cfi_endproc"),
    );
    // RV32I
    #[cfg(all(not(target_feature = "f"), not(target_feature = "e")))]
    core::arch::naked_asm!(
        maybe_cfi!(".cfi_startproc"),
        "
        mv t0, sp
        add sp, sp, -0x90
        ",
        maybe_cfi!(".cfi_def_cfa_offset 0x90"),
        "sw ra, 0x80(sp)",
        maybe_cfi!(".cfi_offset ra, -16"),
        code!(save_gp_0_to_15),
        code!(save_gp_16_to_31),
        "
        mv t0, a0
        mv a0, sp
        jalr t0
        lw ra, 0x80(sp)
        add sp, sp, 0x90
        ",
        maybe_cfi!(".cfi_def_cfa_offset 0"),
        maybe_cfi!(".cfi_restore ra"),
        "ret",
        maybe_cfi!(".cfi_endproc")
    );
    // RV32E
    #[cfg(target_feature = "e")]
    core::arch::naked_asm!(
        maybe_cfi!(".cfi_startproc"),
        "
        mv t0, sp
        add sp, sp, -0x50
        ",
        maybe_cfi!(".cfi_def_cfa_offset 0x50"),
        "sw ra, 0x40(sp)",
        maybe_cfi!(".cfi_offset ra, -16"),
        code!(save_gp_0_to_15),
        "
        mv t0, a0
        mv a0, sp
        jalr t0
        lw ra, 0x40(sp)
        add sp, sp, 0x50
        ",
        maybe_cfi!(".cfi_def_cfa_offset 0"),
        maybe_cfi!(".cfi_restore ra"),
        "ret",
        maybe_cfi!(".cfi_endproc")
    );
}

pub unsafe fn restore_context(ctx: &Context) -> ! {
    // RV32IFD
    #[cfg(target_feature = "d")]
    unsafe {
        core::arch::asm!(
            code!(restore_fp_d),
            code!(restore_gp_0_to_15),
            code!(restore_gp_16_to_31),
            "
            lw a0, 0x28(a0)
            ret
            ",
            in("a0") ctx,
            options(noreturn)
        );
    }
    // RV32IF
    #[cfg(all(target_feature = "f", not(target_feature = "d")))]
    unsafe {
        core::arch::asm!(
            code!(restore_fp_f),
            code!(restore_gp_0_to_15),
            code!(restore_gp_16_to_31),
            "
            lw a0, 0x28(a0)
            ret
            ",
            in("a0") ctx,
            options(noreturn)
        );
    }
    // RV32I
    #[cfg(all(not(target_feature = "f"), not(target_feature = "e")))]
    unsafe {
        core::arch::asm!(
            code!(restore_gp_0_to_15),
            code!(restore_gp_16_to_31),
            "
            lw a0, 0x28(a0)
            ret
            ",
            in("a0") ctx,
            options(noreturn)
        );
    }
    // RV32E
    #[cfg(target_feature = "e")]
    unsafe {
        core::arch::asm!(
            code!(restore_gp_0_to_15),
            "
            lw a0, 0x28(a0)
            ret
            ",
            in("a0") ctx,
            options(noreturn)
        );
    }
}
