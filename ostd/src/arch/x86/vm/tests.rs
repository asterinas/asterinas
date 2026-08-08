// SPDX-License-Identifier: MPL-2.0

use x86::msr::{IA32_APIC_BASE, IA32_EFER, IA32_FS_BASE, IA32_GS_BASE, IA32_PAT};
use x86_64::registers::control::Cr0Flags;

use super::{GuestContext, VcpuRegs, VcpuRunState, X86GprIndex};
use crate::{Error, prelude::*};

#[ktest]
fn bsp_and_ap_start_in_expected_states() {
    let bsp = GuestContext::new(0);
    assert_eq!(bsp.run_state(), VcpuRunState::Runnable);
    assert_eq!(bsp.regs().rflags, 0x2);
    assert_eq!(
        bsp.sregs().cr0,
        (Cr0Flags::EXTENSION_TYPE | Cr0Flags::NUMERIC_ERROR).bits()
    );

    let ap = GuestContext::new(1);
    assert_eq!(ap.run_state(), VcpuRunState::WaitForSipi);
    assert_eq!(ap.regs().rflags, 0x2);
    assert_eq!(
        ap.sregs().cr0,
        (Cr0Flags::EXTENSION_TYPE | Cr0Flags::NUMERIC_ERROR).bits()
    );
}

#[ktest]
fn sipi_initializes_an_ap_only_once() {
    let mut context = GuestContext::new(1);
    let mut regs = context.regs();
    regs.rax = 0x1234;
    regs.rip = 0x5678;
    context.set_regs(regs);
    context.write_msr(IA32_APIC_BASE, 0xfee0_0800).unwrap();

    context.receive_sipi(0x12);
    assert_eq!(context.run_state(), VcpuRunState::Runnable);
    assert_eq!(context.regs().rax, 0);
    assert_eq!(context.rip(), 0);
    assert_eq!(context.sregs().cs.selector, 0x1200);
    assert_eq!(context.sregs().cs.base, 0x12000);
    assert_eq!(context.read_msr(IA32_APIC_BASE).unwrap(), 0xfee0_0800);

    context.receive_sipi(0x34);
    assert_eq!(context.sregs().cs.selector, 0x1200);
    assert_eq!(context.sregs().cs.base, 0x12000);
}

#[ktest]
fn gpr_access_uses_x86_width_semantics() {
    let mut context = GuestContext::new(0);
    context
        .set_gpr(X86GprIndex::Rax, 8, 0x1122_3344_5566_7788)
        .unwrap();
    context.set_gpr(X86GprIndex::Rax, 1, 0xaa).unwrap();
    assert_eq!(context.gpr(X86GprIndex::Rax), 0x1122_3344_5566_77aa);
    context.set_gpr(X86GprIndex::Rax, 2, 0xbbcc).unwrap();
    assert_eq!(context.gpr(X86GprIndex::Rax), 0x1122_3344_5566_bbcc);
    context
        .set_gpr(X86GprIndex::Rax, 4, 0xffff_ffff_dead_beef)
        .unwrap();
    assert_eq!(context.gpr(X86GprIndex::Rax), 0xdead_beef);

    let before = context.regs();
    assert_eq!(
        context.set_gpr(X86GprIndex::Rax, 3, 0),
        Err(Error::InvalidArgs)
    );
    assert_eq!(context.regs(), before);
}

#[ktest]
fn rip_overflow_does_not_change_the_context() {
    let mut context = GuestContext::new(0);
    context.set_regs(VcpuRegs {
        rip: usize::MAX - 1,
        ..VcpuRegs::default()
    });

    context.advance_rip(1).unwrap();
    assert_eq!(context.rip(), usize::MAX);
    assert_eq!(context.advance_rip(1), Err(Error::Overflow));
    assert_eq!(context.rip(), usize::MAX);
}

#[ktest]
fn special_register_and_msr_views_remain_consistent() {
    let mut context = GuestContext::new(0);
    let mut sregs = context.sregs();
    sregs.apic_base = 0xfee0_0900;
    sregs.efer = 0x501;
    sregs.fs.base = 0x1111_2222;
    sregs.gs.base = 0x3333_4444;
    context.set_sregs(sregs);

    assert_eq!(context.read_msr(IA32_APIC_BASE).unwrap(), sregs.apic_base);
    assert_eq!(context.read_msr(IA32_EFER).unwrap(), sregs.efer);
    assert_eq!(context.read_msr(IA32_FS_BASE).unwrap(), sregs.fs.base);
    assert_eq!(context.read_msr(IA32_GS_BASE).unwrap(), sregs.gs.base);

    context.write_msr(IA32_PAT, 0x0102_0304_0506_0708).unwrap();
    assert_eq!(context.read_msr(IA32_PAT).unwrap(), 0x0102_0304_0506_0708);

    context.write_msr(IA32_EFER, 0xd01).unwrap();
    context.write_msr(IA32_FS_BASE, 0x5555_6666).unwrap();
    context.write_msr(IA32_GS_BASE, 0x7777_8888).unwrap();
    assert_eq!(context.sregs().efer, 0xd01);
    assert_eq!(context.sregs().fs.base, 0x5555_6666);
    assert_eq!(context.sregs().gs.base, 0x7777_8888);

    let before = context.sregs();
    assert_eq!(context.read_msr(u32::MAX), Err(Error::InvalidArgs));
    assert_eq!(context.write_msr(u32::MAX, 1), Err(Error::InvalidArgs));
    assert_eq!(context.sregs(), before);
}
