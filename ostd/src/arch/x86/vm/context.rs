use x86_64::registers::{
    control::{Cr0Flags, Cr4Flags},
    model_specific::EferFlags,
};

use super::{
    vmcs::{Vmcs, VmcsGuestState},
    vmx::Msr,
};
use crate::{arch::cpu::context::FpuContext, prelude::*};

pub struct GuestContext {
    /// vcpu id
    id: u32,

    /// gpr, msr, cr, (cs, ss, ..), gdt...
    arch: VcpuArchState,

    /// Running state
    /// is_running, multiprocessor startup state
    run: VcpuRunState,

    /// VMCS
    pub(crate) vmcs: Vmcs,

    pub(crate) tsc_deadline: Option<u64>,
    pub(crate) tsc_offset: i64,

    pub(crate) cpu_config: GuestCpuConfig,

    /// The last vmexit was due to hlt.
    /// After the hlt occurs, when the interrupt causes the vcpu to re-enter,
    /// the "block by sti" bit in the interruptibility state should be cleared first.
    pub(crate) after_hlt: bool,
}

pub(crate) struct VcpuArchState {
    /// General purpose registers
    regs: VcpuRegs,
    /// Special registers and descriptor tables provided by userspace.
    sregs: VcpuSregs,
    /// VMX control-register state split into guest-visible and hardware values.
    control_regs: VcpuControlRegisters,
    /// Guest-visible MSRs emulated by the hypervisor.
    msrs: VcpuMsrs,
    /// FPU/SIMD context.
    fpu: FpuContext,
}

impl GuestContext {
    pub fn new(id: u32) -> Result<Self> {
        Ok(Self {
            id: id,
            arch: VcpuArchState::default(),
            run: if id == 0 {
                VcpuRunState::Runnable
            } else {
                VcpuRunState::WaitForSipi
            },
            vmcs: Vmcs::new()?,
            tsc_deadline: None,
            tsc_offset: 0,
            cpu_config: GuestCpuConfig::default(),
            after_hlt: false,
        })
    }

    pub fn receive_sipi(&mut self, vector: u8) {
        if self.run != VcpuRunState::WaitForSipi {
            return;
        }

        self.arch.regs = VcpuRegs {
            rip: 0,
            rflags: 0x2,
            ..VcpuRegs::default()
        };
        self.arch.set_sregs(VcpuSregs::with_startup(vector));
        self.arch.set_efer(0);
        // self.arch.msrs.tsc_aux = u64::from(vcpu_id);
        self.run = VcpuRunState::Runnable;
    }

    pub fn regs(&self) -> VcpuRegs {
        self.arch.regs
    }

    pub fn set_regs(&mut self, regs: VcpuRegs) {
        self.arch.regs = regs;
    }

    pub fn sregs(&self) -> VcpuSregs {
        self.arch.sregs()
    }

    pub fn set_sregs(&mut self, mut sregs: VcpuSregs) {
        sregs.apic_base = sanitize_apic_base_for_vcpu(sregs.apic_base, self.id);
        self.arch.set_sregs(sregs);
    }

    pub fn gpr(&self, index: u8) -> u64 {
        self.arch.gpr(index)
    }

    pub fn set_gpr(&mut self, index: u8, width_byte: u8, value: u64) {
        self.arch.set_gpr(index, width_byte, value);
    }

    pub fn advance_rip(&mut self, len: u64) {
        self.arch.advance_rip(len);
    }

    pub fn rip(&self) -> u64 {
        self.arch.rip()
    }

    pub fn is_running(&self) -> bool {
        self.run == VcpuRunState::Running
    }

    pub fn guest_tsc(&self) -> u64 {
        use crate::arch::read_tsc;
        let tsc = read_tsc() as i64 + self.tsc_offset;
        if tsc < 0 { 0 } else { tsc as u64 }
    }

    pub(crate) fn tsc_deadline(&self) -> Option<u64> {
        self.tsc_deadline
    }

    // pub fn set_tsc_deadline(&mut self, deadline: Option<u64>) {
    //     self.tsc_deadline = deadline;
    // }

    pub fn set_guest_cpu_config(&mut self, config: GuestCpuConfig) {
        self.cpu_config = config;
    }

    pub(crate) fn vmcs_guest_state(&self) -> VmcsGuestState {
        self.arch.vmcs_guest_state()
    }

    pub(crate) fn arch_mut(&mut self) -> &mut VcpuArchState {
        &mut self.arch
    }

    pub(crate) fn arch(&self) -> &VcpuArchState {
        &self.arch
    }

    pub(crate) fn run_state(&self) -> VcpuRunState {
        self.run
    }

    pub(crate) fn set_running(&mut self) {
        self.run = VcpuRunState::Running;
    }

    pub(crate) fn quit_running(&mut self) {
        self.run = VcpuRunState::Runnable;
    }
}

impl Default for VcpuArchState {
    fn default() -> Self {
        let sregs = VcpuSregs::with_startup(0);
        Self {
            regs: VcpuRegs {
                rflags: 0x2,
                ..VcpuRegs::default()
            },
            sregs,
            control_regs: VcpuControlRegisters::from_sregs(&sregs),
            msrs: VcpuMsrs::default(),
            fpu: FpuContext::new(),
        }
    }
}

impl Default for GuestContext {
    fn default() -> Self {
        Self::new(0).expect("failed to create guest context")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VcpuRunState {
    Running,
    Runnable,
    WaitForSipi,
}

pub(crate) fn sanitize_apic_base_for_vcpu(value: u64, vcpu_id: u32) -> u64 {
    const LAPIC_BASE: u64 = 0xFEE0_0000;
    const APIC_BASE_BSP: u64 = 1 << 8;
    const APIC_BASE_ENABLE: u64 = 1 << 11;

    let bsp = if vcpu_id == 0 { APIC_BASE_BSP } else { 0 };
    LAPIC_BASE | APIC_BASE_ENABLE | bsp | (value & APIC_BASE_BSP)
}

impl Default for VcpuRunState {
    fn default() -> Self {
        Self::Runnable
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct VcpuControlRegisters {
    cr0: VcpuControlRegister,
    cr4: VcpuControlRegister,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VcpuControlRegister {
    host_mask: u64,
    read_shadow: u64,
    real: u64,
}

impl VcpuControlRegisters {
    fn from_sregs(sregs: &VcpuSregs) -> Self {
        Self {
            cr0: VcpuControlRegister::for_cr0_guest_value(sregs.cr0),
            cr4: VcpuControlRegister::for_cr4_guest_value(sregs.cr4),
        }
    }

    pub(crate) fn from_vmcs(cr0: VcpuControlRegister, cr4: VcpuControlRegister) -> Self {
        Self { cr0, cr4 }
    }

    pub(crate) fn cr0(&self) -> VcpuControlRegister {
        self.cr0
    }

    pub(crate) fn cr4(&self) -> VcpuControlRegister {
        self.cr4
    }

    fn write_cr0(&mut self, guest_value: u64) {
        self.cr0 = VcpuControlRegister::for_cr0_guest_value(guest_value);
    }

    fn write_cr4(&mut self, guest_value: u64) {
        self.cr4 = VcpuControlRegister::for_cr4_guest_value(guest_value);
    }
}

impl VcpuControlRegister {
    pub(crate) fn from_vmcs(host_mask: u64, read_shadow: u64, real: u64) -> Self {
        Self {
            host_mask,
            read_shadow,
            real,
        }
    }

    fn for_cr0_guest_value(guest_value: u64) -> Self {
        Self::from_vmcs(cr0_host_mask(), guest_value, cr0_real_value(guest_value))
    }

    fn for_cr4_guest_value(guest_value: u64) -> Self {
        Self::from_vmcs(cr4_host_mask(), guest_value, cr4_real_value(guest_value))
    }

    pub(crate) fn host_mask(&self) -> u64 {
        self.host_mask
    }

    pub(crate) fn read_shadow(&self) -> u64 {
        self.read_shadow
    }

    pub(crate) fn real(&self) -> u64 {
        self.real
    }

    fn guest_value(&self) -> u64 {
        (self.real & !self.host_mask) | (self.read_shadow & self.host_mask)
    }
}

fn cr0_host_mask() -> u64 {
    (Cr0Flags::PROTECTED_MODE_ENABLE
        | Cr0Flags::PAGING
        | Cr0Flags::NUMERIC_ERROR
        | Cr0Flags::NOT_WRITE_THROUGH
        | Cr0Flags::CACHE_DISABLE)
        .bits()
}

fn cr4_host_mask() -> u64 {
    (Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS | Cr4Flags::FSGSBASE).bits()
}

fn cr0_real_value(guest_value: u64) -> u64 {
    let fixed0 = Msr::IA32_VMX_CR0_FIXED0.read();
    let fixed1 = Msr::IA32_VMX_CR0_FIXED1.read();
    let fixed0 = fixed0 & !Cr0Flags::PROTECTED_MODE_ENABLE.bits() & !Cr0Flags::PAGING.bits();
    (guest_value | fixed0) & fixed1
}

fn cr4_real_value(guest_value: u64) -> u64 {
    let fixed0 = Msr::IA32_VMX_CR4_FIXED0.read();
    let fixed1 = Msr::IA32_VMX_CR4_FIXED1.read();
    (guest_value | fixed0 | Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits())
        & (fixed1 & !Cr4Flags::FSGSBASE.bits())
}

#[derive(Debug, Default)]
pub struct GuestCpuConfig {
    pub vcpu_id: u32,
    pub lapic_id: u32,
    pub vcpu_count: u32,
}

impl VcpuArchState {
    pub(crate) fn regs_mut_ptr(&mut self) -> *mut VcpuRegs {
        &mut self.regs
    }

    pub(crate) fn sregs(&self) -> VcpuSregs {
        let mut sregs = self.sregs;
        sregs.cr0 = self.cr0();
        sregs.cr4 = self.cr4();
        sregs
    }

    pub(crate) fn set_sregs(&mut self, sregs: VcpuSregs) {
        self.sregs = sregs;
        self.control_regs = VcpuControlRegisters::from_sregs(&sregs);
        self.msrs.efer = sregs.efer;
        self.msrs.apic_base = sregs.apic_base;
        self.msrs.fs_base = sregs.fs.base;
        self.msrs.gs_base = sregs.gs.base;
        self.sync_efer_lma();
    }

    pub fn gpr(&self, index: u8) -> u64 {
        match index {
            // the order comes from
            // Intel® 64 and IA-32 Architectures Software Developer’s Manual
            // 3.4.1 General-Purpose Registers
            0 => self.regs.rax,
            1 => self.regs.rbx,
            2 => self.regs.rcx,
            3 => self.regs.rdx,
            4 => self.regs.rsi,
            5 => self.regs.rdi,
            6 => self.regs.rbp,
            7 => self.regs.rsp,
            8 => self.regs.r8,
            9 => self.regs.r9,
            10 => self.regs.r10,
            11 => self.regs.r11,
            12 => self.regs.r12,
            13 => self.regs.r13,
            14 => self.regs.r14,
            15 => self.regs.r15,
            _ => 0,
        }
    }

    pub fn set_gpr(&mut self, index: u8, width_byte: u8, value: u64) {
        let slot = match index {
            0 => &mut self.regs.rax,
            1 => &mut self.regs.rbx,
            2 => &mut self.regs.rcx,
            3 => &mut self.regs.rdx,
            4 => &mut self.regs.rsi,
            5 => &mut self.regs.rdi,
            6 => &mut self.regs.rbp,
            7 => &mut self.regs.rsp,
            8 => &mut self.regs.r8,
            9 => &mut self.regs.r9,
            10 => &mut self.regs.r10,
            11 => &mut self.regs.r11,
            12 => &mut self.regs.r12,
            13 => &mut self.regs.r13,
            14 => &mut self.regs.r14,
            15 => &mut self.regs.r15,
            _ => return,
        };

        *slot = match width_byte {
            1 => (*slot & !0xff) | (value & 0xff),
            2 => (*slot & !0xffff) | (value & 0xffff),
            4 => (*slot & !0xffff_ffff) | (value & 0xffff_ffff),
            _ => value,
        };
    }

    pub fn advance_rip(&mut self, len: u64) {
        self.regs.rip += len;
    }

    pub(crate) fn rip(&self) -> u64 {
        self.regs.rip
    }

    pub(crate) fn set_rip(&mut self, value: u64) {
        self.regs.rip = value;
    }

    pub(crate) fn rflags(&self) -> u64 {
        self.regs.rflags
    }

    pub(crate) fn set_rflags(&mut self, value: u64) {
        self.regs.rflags = value;
    }

    pub(crate) fn msr(&self, index: u32) -> u64 {
        use x86::msr::*;
        match index {
            IA32_TSC_ADJUST => self.msrs.tsc_adjust,
            IA32_APIC_BASE => self.msrs.apic_base,
            IA32_SYSENTER_CS => self.msrs.sysenter_cs,
            IA32_SYSENTER_ESP => self.msrs.sysenter_esp,
            IA32_SYSENTER_EIP => self.msrs.sysenter_eip,
            IA32_EFER => self.msrs.efer,
            IA32_PAT => self.msrs.pat,
            IA32_FS_BASE => self.msrs.fs_base,
            IA32_GS_BASE => self.msrs.gs_base,
            IA32_KERNEL_GSBASE => self.msrs.kernel_gs_base,
            IA32_TSC_AUX => self.msrs.tsc_aux,
            IA32_STAR => self.msrs.star,
            IA32_LSTAR => self.msrs.lstar,
            IA32_CSTAR => self.msrs.cstar,
            IA32_FMASK => self.msrs.syscall_mask,
            IA32_TSC_DEADLINE => self.msrs.tsc_deadline,
            _ => {
                error!("get unknown msr {:x}, return 0.", index);
                0
            }
        }
    }

    pub(crate) fn set_msr(&mut self, index: u32, value: u64) {
        use x86::msr::*;
        match index {
            IA32_TSC_ADJUST => self.msrs.tsc_adjust = value,
            IA32_APIC_BASE => {
                self.msrs.apic_base = value;
                self.sregs.apic_base = value;
            }
            IA32_SYSENTER_CS => self.msrs.sysenter_cs = value,
            IA32_SYSENTER_ESP => self.msrs.sysenter_esp = value,
            IA32_SYSENTER_EIP => self.msrs.sysenter_eip = value,
            IA32_EFER => self.set_efer(value),
            IA32_PAT => self.msrs.pat = value,
            IA32_KERNEL_GSBASE => self.msrs.kernel_gs_base = value,
            IA32_TSC_AUX => self.msrs.tsc_aux = value,
            IA32_STAR => self.msrs.star = value,
            IA32_LSTAR => self.msrs.lstar = value,
            IA32_CSTAR => self.msrs.cstar = value,
            IA32_FMASK => self.msrs.syscall_mask = value,
            IA32_TSC_DEADLINE => self.msrs.tsc_deadline = value,
            IA32_FS_BASE => self.set_fs_base(value),
            IA32_GS_BASE => self.set_gs_base(value),
            _ => error!("set_msr: msr {:x} not impl.", index),
        }
    }

    pub(crate) fn cr0(&self) -> u64 {
        self.control_regs.cr0.guest_value()
    }

    pub(crate) fn cr2(&self) -> u64 {
        self.sregs.cr2
    }

    pub(crate) fn cr3(&self) -> u64 {
        self.sregs.cr3
    }

    pub(crate) fn cr4(&self) -> u64 {
        self.control_regs.cr4.guest_value()
    }

    pub(crate) fn control_regs(&self) -> VcpuControlRegisters {
        self.control_regs
    }

    pub(crate) fn set_control_regs_from_vmcs(&mut self, control_regs: VcpuControlRegisters) {
        self.control_regs = control_regs;
        self.sregs.cr0 = self.control_regs.cr0.guest_value();
        self.sregs.cr4 = self.control_regs.cr4.guest_value();
    }

    pub(crate) fn write_cr0(&mut self, value: u64) {
        self.control_regs.write_cr0(value);
        self.sregs.cr0 = self.control_regs.cr0.guest_value();
        self.sync_efer_lma();
    }

    pub(crate) fn set_cr2(&mut self, value: u64) {
        self.sregs.cr2 = value;
    }

    pub(crate) fn set_cr3(&mut self, value: u64) {
        self.sregs.cr3 = value;
    }

    pub(crate) fn write_cr4(&mut self, value: u64) {
        self.control_regs.write_cr4(value);
        self.sregs.cr4 = self.control_regs.cr4.guest_value();
    }

    pub(crate) fn set_cs(&mut self, segment: VcpuSegment) {
        self.sregs.cs = segment;
    }

    pub(crate) fn set_ds(&mut self, segment: VcpuSegment) {
        self.sregs.ds = segment;
    }

    pub(crate) fn set_es(&mut self, segment: VcpuSegment) {
        self.sregs.es = segment;
    }

    pub(crate) fn set_fs(&mut self, segment: VcpuSegment) {
        self.sregs.fs = segment;
        self.msrs.fs_base = segment.base;
    }

    pub(crate) fn set_gs(&mut self, segment: VcpuSegment) {
        self.sregs.gs = segment;
        self.msrs.gs_base = segment.base;
    }

    pub(crate) fn set_ss(&mut self, segment: VcpuSegment) {
        self.sregs.ss = segment;
    }

    pub(crate) fn set_tr(&mut self, segment: VcpuSegment) {
        self.sregs.tr = segment;
    }

    pub(crate) fn set_ldt(&mut self, segment: VcpuSegment) {
        self.sregs.ldt = segment;
    }

    pub(crate) fn set_gdt(&mut self, table: VcpuDtable) {
        self.sregs.gdt = table;
    }

    pub(crate) fn set_idt(&mut self, table: VcpuDtable) {
        self.sregs.idt = table;
    }

    pub(crate) fn set_fs_base(&mut self, value: u64) {
        self.sregs.fs.base = value;
        self.msrs.fs_base = value;
    }

    pub(crate) fn set_gs_base(&mut self, value: u64) {
        self.sregs.gs.base = value;
        self.msrs.gs_base = value;
    }

    pub(crate) fn set_efer(&mut self, value: u64) {
        self.msrs.efer = value;
        self.sync_efer_lma();
    }

    fn sync_efer_lma(&mut self) {
        if (self.msrs.efer & EferFlags::LONG_MODE_ENABLE.bits()) != 0
            && (self.cr0() & Cr0Flags::PAGING.bits()) != 0
        {
            self.msrs.efer |= EferFlags::LONG_MODE_ACTIVE.bits();
        } else {
            self.msrs.efer &= !EferFlags::LONG_MODE_ACTIVE.bits();
        }
        self.sregs.efer = self.msrs.efer;
    }

    pub(crate) fn load_fpu(&mut self) {
        self.fpu.load();
    }

    pub(crate) fn save_fpu(&mut self) {
        self.fpu.save();
    }

    fn vmcs_guest_state(&self) -> VmcsGuestState {
        VmcsGuestState {
            regs: self.regs,
            sregs: self.sregs,
            control_regs: self.control_regs,
            msrs: self.msrs,
        }
    }
}

/// Guest general purpose registers
///
/// This structure represents the guest CPU's general purpose registers
/// that need to be saved/restored during VM entry/exit.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub struct VcpuRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct VcpuMsrs {
    pub apic_base: u64,
    pub efer: u64,
    pub pat: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub kernel_gs_base: u64,
    pub star: u64,
    pub lstar: u64,
    pub cstar: u64,
    pub syscall_mask: u64,
    pub tsc_deadline: u64,
    pub tsc_adjust: u64,
    pub tsc_aux: u64,
    pub sysenter_cs: u64,
    pub sysenter_esp: u64,
    pub sysenter_eip: u64,
}

impl Default for VcpuMsrs {
    fn default() -> Self {
        Self {
            apic_base: 0,
            efer: 0,
            pat: 0x0007_0406_0007_0406,
            fs_base: 0,
            gs_base: 0,
            kernel_gs_base: 0,
            star: 0,
            lstar: 0,
            cstar: 0,
            syscall_mask: 0,
            tsc_deadline: 0,
            tsc_adjust: 0,
            tsc_aux: 0,
            sysenter_cs: 0,
            sysenter_esp: 0,
            sysenter_eip: 0,
        }
    }
}

/// Guest special register state.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub struct VcpuSregs {
    pub cs: VcpuSegment,
    pub ds: VcpuSegment,
    pub es: VcpuSegment,
    pub fs: VcpuSegment,
    pub gs: VcpuSegment,
    pub ss: VcpuSegment,
    pub tr: VcpuSegment,
    pub ldt: VcpuSegment,
    pub gdt: VcpuDtable,
    pub idt: VcpuDtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; 4],
}

/// Guest segment register state.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub struct VcpuSegment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
    pub unusable: u8,
    pub padding: u8,
}

/// Guest descriptor table state.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub struct VcpuDtable {
    pub base: u64,
    pub limit: u16,
    pub padding: [u16; 3],
}

impl VcpuSregs {
    fn with_startup(startup_vector: u8) -> Self {
        let code_base = u64::from(startup_vector) << 12;
        let code_selector = u16::from(startup_vector) << 8;
        let data = VcpuSegment::real_mode_data_segment(0, 0);

        Self {
            cs: VcpuSegment::real_mode_code_segment(code_selector, code_base),
            ds: data,
            es: data,
            fs: data,
            gs: data,
            ss: data,
            tr: VcpuSegment::real_mode_system_segment(0x20, 0, 0x0b),
            ldt: VcpuSegment {
                unusable: 1,
                ..VcpuSegment::default()
            },
            cr0: (Cr0Flags::EXTENSION_TYPE | Cr0Flags::NUMERIC_ERROR).bits(),
            ..VcpuSregs::default()
        }
    }
}

impl VcpuSegment {
    fn real_mode_code_segment(selector: u16, base: u64) -> Self {
        Self::real_mode_segment(selector, base, 0x0b, 1)
    }

    fn real_mode_data_segment(selector: u16, base: u64) -> Self {
        Self::real_mode_segment(selector, base, 0x03, 1)
    }

    fn real_mode_system_segment(selector: u16, base: u64, type_: u8) -> Self {
        Self::real_mode_segment(selector, base, type_, 0)
    }

    fn real_mode_segment(selector: u16, base: u64, type_: u8, s: u8) -> Self {
        VcpuSegment {
            base,
            limit: 0xffff,
            selector,
            type_,
            present: 1,
            dpl: 0,
            db: 0,
            s,
            l: 0,
            g: 0,
            avl: 0,
            unusable: 0,
            padding: 0,
        }
    }
}
