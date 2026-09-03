// SPDX-License-Identifier: MPL-2.0

use x86::msr::{
    IA32_APIC_BASE, IA32_EFER, IA32_FS_BASE, IA32_GS_BASE, IA32_PAT, IA32_SYSENTER_EIP,
};
use x86_64::registers::control::Cr0Flags;

use super::{GuestContext, VcpuRegs, VcpuRunState, X86GprIndex};
use crate::{Error, prelude::*};

#[ktest]
fn bsp_and_ap_start_in_expected_states() {
    let bsp = GuestContext::new(0);
    assert_eq!(bsp.run_state(), VcpuRunState::Runnable);
    assert!(!bsp.run_state().waits_for_startup());
    assert_eq!(bsp.regs().rflags, 0x2);
    assert_eq!(bsp.regs().rdx, 0x600);
    assert_eq!(bsp.rip(), 0xfff0);
    assert_eq!(bsp.sregs().cs.selector, 0xf000);
    assert_eq!(bsp.sregs().cs.base, 0xffff_0000);
    assert_eq!(bsp.sregs().gdt.limit, 0xffff);
    assert_eq!(bsp.sregs().idt.limit, 0xffff);
    assert_eq!(
        bsp.sregs().cr0,
        (Cr0Flags::CACHE_DISABLE | Cr0Flags::NOT_WRITE_THROUGH | Cr0Flags::EXTENSION_TYPE).bits()
    );
    assert_eq!(bsp.sregs().apic_base, 0xfee0_0900);

    let ap = GuestContext::new(1);
    assert_eq!(ap.run_state(), VcpuRunState::Uninitialized);
    assert!(ap.run_state().waits_for_startup());
    assert_eq!(ap.regs().rflags, 0x2);
    assert_eq!(ap.rip(), 0xfff0);
    assert_eq!(ap.sregs().apic_base, 0xfee0_0800);
}

#[ktest]
fn init_and_sipi_follow_x86_state_transitions() {
    const PROCESSOR_SIGNATURE: u32 = 0x0006_06a1;
    const TEST_PAT: u64 = 0x0001_0203_0405_0607;
    const TEST_SYSENTER_EIP: u64 = 0x1234_5678;

    let mut ap = GuestContext::new(1);
    ap.receive_sipi(0x07);
    assert_eq!(ap.run_state(), VcpuRunState::Uninitialized);
    assert_eq!(ap.rip(), 0xfff0);

    let apic_base = ap.sregs().apic_base;
    let mut sregs = ap.sregs();
    sregs.cr0 = (sregs.cr0 | (1 << 31)) & !(1 << 29);
    sregs.cr3 = 0x4000;
    sregs.cr4 = 0x20;
    sregs.efer = 0x500;
    ap.set_sregs(sregs);
    ap.write_msr(IA32_PAT, TEST_PAT).unwrap();
    ap.write_msr(IA32_SYSENTER_EIP, TEST_SYSENTER_EIP).unwrap();

    ap.receive_init(PROCESSOR_SIGNATURE);
    assert_eq!(ap.run_state(), VcpuRunState::WaitForSipi);
    assert!(ap.run_state().waits_for_startup());
    assert_eq!(ap.rip(), 0xfff0);
    assert_eq!(ap.regs().rdx, PROCESSOR_SIGNATURE as usize);
    assert_eq!(ap.sregs().cs.selector, 0xf000);
    assert_eq!(ap.sregs().cs.base, 0xffff_0000);
    assert_eq!(ap.sregs().cr0, 0x4000_0010);
    assert_eq!(ap.sregs().cr3, 0);
    assert_eq!(ap.sregs().cr4, 0);
    assert_eq!(ap.sregs().efer, 0);
    assert_eq!(ap.sregs().apic_base, apic_base);
    assert_eq!(ap.read_msr(IA32_PAT).unwrap(), TEST_PAT);
    assert_eq!(ap.read_msr(IA32_SYSENTER_EIP).unwrap(), TEST_SYSENTER_EIP);

    let mut regs = ap.regs();
    regs.rax = 0xfeed_face;
    ap.set_regs(regs);
    let mut sregs = ap.sregs();
    sregs.ds.selector = 0x20;
    sregs.ds.base = 0x200;
    ap.set_sregs(sregs);

    ap.receive_sipi(0x08);
    assert_eq!(ap.run_state(), VcpuRunState::Runnable);
    assert_eq!(ap.rip(), 0);
    assert_eq!(ap.sregs().cs.selector, 0x0800);
    assert_eq!(ap.sregs().cs.base, 0x8000);
    assert_eq!(ap.regs().rax, 0xfeed_face);
    assert_eq!(ap.regs().rdx, PROCESSOR_SIGNATURE as usize);
    assert_eq!(ap.sregs().ds.selector, 0x20);
    assert_eq!(ap.sregs().ds.base, 0x200);
    assert_eq!(ap.read_msr(IA32_PAT).unwrap(), TEST_PAT);
    assert_eq!(ap.read_msr(IA32_SYSENTER_EIP).unwrap(), TEST_SYSENTER_EIP);

    ap.receive_sipi(0x09);
    assert_eq!(ap.sregs().cs.selector, 0x0800);
    assert_eq!(ap.sregs().cs.base, 0x8000);

    let mut bsp = GuestContext::new(0);
    bsp.receive_init(PROCESSOR_SIGNATURE);
    assert_eq!(bsp.run_state(), VcpuRunState::Runnable);
    assert_eq!(bsp.rip(), 0xfff0);
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
