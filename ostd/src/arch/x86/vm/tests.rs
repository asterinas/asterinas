// SPDX-License-Identifier: MPL-2.0

use core::mem::{offset_of, size_of};

use x86::vmx::vmcs::control::{
    EntryControls, ExitControls, PinbasedControls, PrimaryControls, SecondaryControls,
};
use x86_64::registers::{
    control::{Cr3, Cr4, Cr4Flags},
    model_specific::EferFlags,
};

use super::{
    control_regs::{VcpuControlRegister, VcpuControlRegisters},
    types::{VcpuDtable, VcpuMsrs, VcpuRegs, VcpuSegment, VcpuSregs},
    vmcs::{self, Vmcs, VmcsGuestState},
    vmx::{
        self, Msr, VmcsControl32, VmcsControl64, VmcsControlNW, VmcsGuest16, VmcsGuest32,
        VmcsGuest64, VmcsGuestNW, VmcsHostNW,
    },
};
use crate::{
    prelude::*,
    task::{self, DisabledPreemptGuard},
};

struct VmxTestGuard {
    old_cr4: u64,
    _preempt_guard: DisabledPreemptGuard,
}

impl VmxTestGuard {
    fn new() -> Option<Self> {
        if !platform_can_enter_vmx() {
            return None;
        }

        let preempt_guard = task::disable_preempt();
        let old_cr4 = Cr4::read_raw();
        vmx::test_support::init_vmcs_revision();

        // SAFETY: The test pins itself to the current CPU by disabling
        // preemption and restores the original CR4 value in `Drop`.
        unsafe {
            Cr4::write_raw(old_cr4 | Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits());
        }

        if vmx::test_support::vmxon_current_cpu().is_err() {
            // SAFETY: Restores the CR4 value saved above.
            unsafe {
                Cr4::write_raw(old_cr4);
            }
            return None;
        }

        Some(Self {
            old_cr4,
            _preempt_guard: preempt_guard,
        })
    }
}

impl Drop for VmxTestGuard {
    fn drop(&mut self) {
        let _ = vmx::vmxoff();
        // SAFETY: Restores the CR4 value saved before the test entered VMX
        // operation on this CPU.
        unsafe {
            Cr4::write_raw(self.old_cr4);
        }
    }
}

struct CurrentVmcs {
    paddr: u64,
}

impl Drop for CurrentVmcs {
    fn drop(&mut self) {
        let _ = vmx::vmclear(self.paddr);
    }
}

fn platform_can_enter_vmx() -> bool {
    const IA32_FEATURE_CONTROL_LOCK: u64 = 1;
    const IA32_FEATURE_CONTROL_VMX_OUTSIDE_SMX: u64 = 1 << 2;
    const TRUE_CONTROLS_AVAILABLE: u64 = 1 << 55;

    let cpuid = core::arch::x86_64::__cpuid(1);
    if (cpuid.ecx & (1 << 5)) == 0 {
        return false;
    }

    let feature_control = Msr::IA32_FEATURE_CONTROL.read();
    if feature_control & (IA32_FEATURE_CONTROL_LOCK | IA32_FEATURE_CONTROL_VMX_OUTSIDE_SMX)
        != (IA32_FEATURE_CONTROL_LOCK | IA32_FEATURE_CONTROL_VMX_OUTSIDE_SMX)
    {
        return false;
    }

    Msr::IA32_VMX_BASIC.read() & TRUE_CONTROLS_AVAILABLE != 0
}

fn vmcs_init_controls_are_supported() -> bool {
    let supports_pause_loop_exiting = controls_can_set(
        Msr::IA32_VMX_PROCBASED_CTLS2,
        SecondaryControls::PAUSE_LOOP_EXITING.bits(),
    );
    let pause_exiting_fallback = if supports_pause_loop_exiting {
        0
    } else {
        PrimaryControls::PAUSE_EXITING.bits()
    };
    let pause_loop_exiting = if supports_pause_loop_exiting {
        SecondaryControls::PAUSE_LOOP_EXITING.bits()
    } else {
        0
    };

    controls_can_set(
        Msr::IA32_VMX_TRUE_PINBASED_CTLS,
        (PinbasedControls::EXTERNAL_INTERRUPT_EXITING
            | PinbasedControls::NMI_EXITING
            | PinbasedControls::VMX_PREEMPTION_TIMER)
            .bits(),
    ) && controls_can_set(
        Msr::IA32_VMX_TRUE_PROCBASED_CTLS,
        (PrimaryControls::USE_TSC_OFFSETTING
            | PrimaryControls::HLT_EXITING
            | PrimaryControls::USE_IO_BITMAPS
            | PrimaryControls::USE_MSR_BITMAPS
            | PrimaryControls::SECONDARY_CONTROLS)
            .bits()
            | pause_exiting_fallback,
    ) && controls_can_clear(
        Msr::IA32_VMX_TRUE_PROCBASED_CTLS,
        (PrimaryControls::CR3_LOAD_EXITING | PrimaryControls::CR3_STORE_EXITING).bits(),
    ) && controls_can_set(
        Msr::IA32_VMX_PROCBASED_CTLS2,
        (SecondaryControls::ENABLE_EPT
            | SecondaryControls::ENABLE_RDTSCP
            | SecondaryControls::UNRESTRICTED_GUEST)
            .bits()
            | pause_loop_exiting,
    ) && controls_can_set(
        Msr::IA32_VMX_TRUE_EXIT_CTLS,
        (ExitControls::HOST_ADDRESS_SPACE_SIZE
            | ExitControls::SAVE_IA32_PAT
            | ExitControls::LOAD_IA32_PAT
            | ExitControls::SAVE_IA32_EFER
            | ExitControls::LOAD_IA32_EFER)
            .bits(),
    ) && controls_can_set(
        Msr::IA32_VMX_TRUE_ENTRY_CTLS,
        (EntryControls::LOAD_IA32_PAT
            | EntryControls::LOAD_IA32_EFER
            | EntryControls::IA32E_MODE_GUEST)
            .bits(),
    )
}

fn controls_can_set(capability_msr: Msr, bits: u32) -> bool {
    let cap = capability_msr.read();
    let allowed1 = (cap >> 32) as u32;
    allowed1 & bits == bits
}

fn controls_can_clear(capability_msr: Msr, bits: u32) -> bool {
    let cap = capability_msr.read();
    let allowed0 = cap as u32;
    allowed0 & bits == 0
}

fn code_segment() -> VcpuSegment {
    VcpuSegment {
        base: 0,
        limit: 0x000f_ffff,
        selector: 0x8,
        type_: 0b1011,
        present: 1,
        dpl: 0,
        db: 0,
        s: 1,
        l: 1,
        g: 1,
        ..VcpuSegment::default()
    }
}

fn data_segment(selector: u16) -> VcpuSegment {
    VcpuSegment {
        base: 0,
        limit: 0x000f_ffff,
        selector,
        type_: 0b0011,
        present: 1,
        dpl: 0,
        db: 1,
        s: 1,
        g: 1,
        ..VcpuSegment::default()
    }
}

fn unusable_segment() -> VcpuSegment {
    VcpuSegment {
        unusable: 1,
        ..VcpuSegment::default()
    }
}

fn test_guest_state() -> VmcsGuestState {
    let cs = code_segment();
    let ds = data_segment(0x10);
    let tr = VcpuSegment {
        base: 0x7000,
        limit: 0x67,
        selector: 0x18,
        type_: 0b1011,
        present: 1,
        ..VcpuSegment::default()
    };
    let sregs = VcpuSregs {
        cs,
        ds,
        es: ds,
        fs: ds,
        gs: ds,
        ss: ds,
        tr,
        ldt: unusable_segment(),
        gdt: VcpuDtable {
            base: 0x1000,
            limit: 0x30,
            padding: [0; 3],
        },
        idt: VcpuDtable {
            base: 0x2000,
            limit: 0x100,
            padding: [0; 3],
        },
        cr3: 0x3000,
        ..VcpuSregs::default()
    };
    let control_regs = VcpuControlRegisters::from_vmcs(
        VcpuControlRegister::from_vmcs(0xff0f, 0x21, 0x8000_0031),
        VcpuControlRegister::from_vmcs(0x20f0, 0x20, Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits()),
    );
    let regs = VcpuRegs {
        rsp: 0x8000,
        rip: 0x100000,
        rflags: 0,
        ..VcpuRegs::default()
    };
    let msrs = VcpuMsrs {
        efer: EferFlags::LONG_MODE_ENABLE.bits() | EferFlags::LONG_MODE_ACTIVE.bits(),
        sysenter_cs: 0x33,
        sysenter_esp: 0x4444,
        sysenter_eip: 0x5555,
        ..VcpuMsrs::default()
    };

    VmcsGuestState {
        regs,
        sregs,
        control_regs,
        msrs,
    }
}

mod vcpu_state_layout {
    use super::*;

    #[ktest]
    fn regs_layout_matches_assembly_offsets() {
        assert_eq!(offset_of!(VcpuRegs, rax), 0x00);
        assert_eq!(offset_of!(VcpuRegs, rbx), 0x08);
        assert_eq!(offset_of!(VcpuRegs, rcx), 0x10);
        assert_eq!(offset_of!(VcpuRegs, rdx), 0x18);
        assert_eq!(offset_of!(VcpuRegs, rsi), 0x20);
        assert_eq!(offset_of!(VcpuRegs, rdi), 0x28);
        assert_eq!(offset_of!(VcpuRegs, rbp), 0x30);
        assert_eq!(offset_of!(VcpuRegs, rsp), 0x38);
        assert_eq!(offset_of!(VcpuRegs, r8), 0x40);
        assert_eq!(offset_of!(VcpuRegs, r9), 0x48);
        assert_eq!(offset_of!(VcpuRegs, r10), 0x50);
        assert_eq!(offset_of!(VcpuRegs, r11), 0x58);
        assert_eq!(offset_of!(VcpuRegs, r12), 0x60);
        assert_eq!(offset_of!(VcpuRegs, r13), 0x68);
        assert_eq!(offset_of!(VcpuRegs, r14), 0x70);
        assert_eq!(offset_of!(VcpuRegs, r15), 0x78);
        assert_eq!(offset_of!(VcpuRegs, rip), 0x80);
        assert_eq!(offset_of!(VcpuRegs, rflags), 0x88);
        assert_eq!(size_of::<VcpuRegs>(), 0x90);
    }
}

mod control_registers {
    use super::*;

    #[ktest]
    fn control_register_shadow_reconstructs_guest_value() {
        let register = VcpuControlRegister::from_vmcs(0xf0, 0xa0, 0x5a);

        assert_eq!(register.host_mask(), 0xf0);
        assert_eq!(register.read_shadow(), 0xa0);
        assert_eq!(register.real(), 0x5a);
        assert_eq!(register.guest_value(), 0xaa);
    }
}

mod vmx_instructions {
    use super::*;

    #[ktest]
    fn vmwrite_then_vmread_observes_current_vmcs() {
        let Some(_vmx_guard) = VmxTestGuard::new() else {
            return;
        };
        let vmcs = vmx::alloc_vmcs().unwrap();
        let vmcs_paddr = vmcs.paddr() as u64;

        vmx::vmclear(vmcs_paddr).unwrap();
        vmx::vmptrld(vmcs_paddr).unwrap();
        let current_vmcs = CurrentVmcs { paddr: vmcs_paddr };

        VmcsGuestNW::RIP.write(0x1234_5678).unwrap();
        VmcsGuestNW::RSP.write(0x8765_4321).unwrap();
        VmcsGuestNW::RFLAGS.write(0x202).unwrap();
        VmcsControl64::TSC_OFFSET
            .write(0x1122_3344_5566_7788)
            .unwrap();
        VmcsHostNW::RSP.write(0x1357_2468).unwrap();

        assert_eq!(VmcsGuestNW::RIP.read().unwrap(), 0x1234_5678);
        assert_eq!(VmcsGuestNW::RSP.read().unwrap(), 0x8765_4321);
        assert_eq!(VmcsGuestNW::RFLAGS.read().unwrap(), 0x202);
        assert_eq!(
            VmcsControl64::TSC_OFFSET.read().unwrap(),
            0x1122_3344_5566_7788
        );
        assert_eq!(VmcsHostNW::RSP.read().unwrap(), 0x1357_2468);

        drop(current_vmcs);
    }
}

mod vmcs_initialization {
    use super::*;

    #[ktest]
    fn vmcs_init_writes_guest_state_and_controls() {
        let Some(_vmx_guard) = VmxTestGuard::new() else {
            return;
        };
        if !vmcs_init_controls_are_supported() {
            return;
        }

        let guest_state = test_guest_state();
        let regs = guest_state.regs;
        let sregs = guest_state.sregs;
        let control_regs = guest_state.control_regs;
        let msrs = guest_state.msrs;
        let eptp = 0x1e;
        let mut vmcs = Vmcs::new().unwrap();

        vmcs.init(guest_state, eptp).unwrap();

        assert!(vmcs.initialized());
        assert!(!vmcs.launched());
        assert_eq!(VmcsGuestNW::RSP.read().unwrap(), regs.rsp as usize);
        assert_eq!(VmcsGuestNW::RIP.read().unwrap(), regs.rip as usize);
        assert_eq!(VmcsGuestNW::RFLAGS.read().unwrap(), 0x2);

        assert_eq!(
            VmcsGuestNW::CR0.read().unwrap(),
            control_regs.cr0().real() as usize
        );
        assert_eq!(
            VmcsControlNW::CR0_GUEST_HOST_MASK.read().unwrap(),
            control_regs.cr0().host_mask() as usize
        );
        assert_eq!(
            VmcsControlNW::CR0_READ_SHADOW.read().unwrap(),
            control_regs.cr0().read_shadow() as usize
        );
        assert_eq!(
            VmcsGuestNW::CR4.read().unwrap(),
            control_regs.cr4().real() as usize
        );
        assert_eq!(
            VmcsControlNW::CR4_GUEST_HOST_MASK.read().unwrap(),
            control_regs.cr4().host_mask() as usize
        );
        assert_eq!(
            VmcsControlNW::CR4_READ_SHADOW.read().unwrap(),
            control_regs.cr4().read_shadow() as usize
        );

        assert_eq!(VmcsGuest16::CS_SELECTOR.read().unwrap(), sregs.cs.selector);
        assert_eq!(VmcsGuestNW::CS_BASE.read().unwrap(), sregs.cs.base as usize);
        assert_eq!(VmcsGuest32::CS_LIMIT.read().unwrap(), sregs.cs.limit);
        assert_eq!(
            VmcsGuest32::CS_ACCESS_RIGHTS.read().unwrap(),
            vmcs::segment_access_rights(&sregs.cs)
        );
        assert_eq!(VmcsGuest16::TR_SELECTOR.read().unwrap(), sregs.tr.selector);
        assert_eq!(VmcsGuestNW::TR_BASE.read().unwrap(), sregs.tr.base as usize);
        assert_eq!(
            VmcsGuest32::TR_ACCESS_RIGHTS.read().unwrap(),
            vmcs::segment_access_rights(&sregs.tr)
        );
        assert_eq!(
            VmcsGuest32::LDTR_ACCESS_RIGHTS.read().unwrap(),
            vmcs::segment_access_rights(&sregs.ldt)
        );

        assert_eq!(
            VmcsGuestNW::GDTR_BASE.read().unwrap(),
            sregs.gdt.base as usize
        );
        assert_eq!(
            VmcsGuest32::GDTR_LIMIT.read().unwrap(),
            u32::from(sregs.gdt.limit)
        );
        assert_eq!(
            VmcsGuestNW::IDTR_BASE.read().unwrap(),
            sregs.idt.base as usize
        );
        assert_eq!(
            VmcsGuest32::IDTR_LIMIT.read().unwrap(),
            u32::from(sregs.idt.limit)
        );
        assert_eq!(VmcsGuestNW::CR3.read().unwrap(), sregs.cr3 as usize);

        assert_eq!(VmcsGuest64::IA32_PAT.read().unwrap(), msrs.pat);
        assert_eq!(VmcsGuest64::IA32_EFER.read().unwrap(), msrs.efer);
        assert_eq!(
            VmcsGuest32::IA32_SYSENTER_CS.read().unwrap(),
            msrs.sysenter_cs as u32
        );
        assert_eq!(
            VmcsGuestNW::IA32_SYSENTER_ESP.read().unwrap(),
            msrs.sysenter_esp as usize
        );
        assert_eq!(
            VmcsGuestNW::IA32_SYSENTER_EIP.read().unwrap(),
            msrs.sysenter_eip as usize
        );

        assert_eq!(VmcsControl64::EPTP.read().unwrap(), eptp);
        assert_eq!(VmcsControl32::VMEXIT_MSR_STORE_COUNT.read().unwrap(), 0);
        assert_eq!(VmcsControl32::VMEXIT_MSR_LOAD_COUNT.read().unwrap(), 0);
        assert_eq!(VmcsControl32::VMENTRY_MSR_LOAD_COUNT.read().unwrap(), 0);
        assert_eq!(VmcsControl32::EXCEPTION_BITMAP.read().unwrap(), 0);
        assert_ne!(VmcsControl64::IO_BITMAP_A_ADDR.read().unwrap(), 0);
        assert_ne!(VmcsControl64::IO_BITMAP_B_ADDR.read().unwrap(), 0);
        assert_ne!(VmcsControl64::MSR_BITMAPS_ADDR.read().unwrap(), 0);

        let entry_controls = VmcsControl32::VMENTRY_CONTROLS.read().unwrap();
        assert_ne!(entry_controls & EntryControls::IA32E_MODE_GUEST.bits(), 0);

        assert_eq!(
            VmcsHostNW::CR3.read().unwrap(),
            Cr3::read_raw().0.start_address().as_u64() as usize
        );
        assert_eq!(
            VmcsHostNW::RIP.read().unwrap(),
            vmx::vm_exit_handler_virtaddr()
        );
    }

    #[ktest]
    fn segment_access_rights_uses_vmx_bit_layout() {
        let segment = VcpuSegment {
            type_: 0b1011,
            present: 1,
            dpl: 3,
            db: 0,
            s: 1,
            l: 1,
            g: 1,
            avl: 1,
            unusable: 0,
            ..VcpuSegment::default()
        };

        let expected = 0b1011 | (1 << 4) | (3 << 5) | (1 << 7) | (1 << 12) | (1 << 13) | (1 << 15);
        assert_eq!(vmcs::segment_access_rights(&segment), expected);
        assert_eq!(vmcs::segment_access_rights(&unusable_segment()), 1 << 16);
    }
}
