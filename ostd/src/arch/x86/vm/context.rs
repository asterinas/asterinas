use core::arch::x86_64::CpuidResult;

use x86_64::registers::{
    control::{Cr0Flags, Cr4Flags},
    model_specific::EferFlags,
};

use super::{
    vmcs::{Vmcs, VmcsGuestState},
    vmx::Msr,
};
use crate::{
    arch::{
        cpu::{context::FpuContext, cpuid::cpuid},
        tsc_freq,
    },
    prelude::*,
};

/// The CPUID entry matches the `ECX` subleaf.
pub const GUEST_CPUID_FLAG_SIGNIFICANT_INDEX: u32 = 1 << 0;

/// Stores the execution context and run state of a guest vCPU.
///
/// The kernel uses it to configure the vCPU-visible context, including
/// general-purpose registers, special registers, MSRs, CPUID leaves, and
/// topology.
///
/// OSTD uses it to emulate guest instructions and to provide
/// [`crate::vm::GuestMode`] with the state needed to run the vCPU. Before
/// entering the vCPU, `GuestMode` loads the context into hardware. After a
/// VM exit, `GuestMode` synchronizes the hardware vCPU state back into this
/// context.
///
/// Setters on this type preserve internal context consistency. For example,
/// updating `CR0` or `EFER` keeps `EFER.LMA` consistent with `EFER.LME` and
/// `CR0.PG`. They do not prove that every guest-supplied value is
/// architecturally useful or bootable. The kernel remains responsible for
/// providing sensible `RIP`, general-purpose register, segment, control
/// register, `MSR`, and `CPUID` values for the guest it intends to run.
pub struct GuestContext {
    /// The vCPU ID.
    id: u32,

    /// The guest architectural state.
    arch: VcpuArchState,

    /// The vCPU run state.
    run: VcpuRunState,

    /// The VMCS owned by this vCPU.
    pub(crate) vmcs: Vmcs,

    pub(crate) tsc_deadline: Option<u64>,
    pub(crate) tsc_offset: i64,

    pub(crate) cpu_config: GuestCpuConfig,

    /// The last VM exit was due to `HLT`.
    /// After `HLT`, when an interrupt causes the vCPU to re-enter, the
    /// block-by-`STI` bit in the interruptibility state should be cleared
    /// first.
    pub(crate) after_hlt: bool,
}

pub(crate) struct VcpuArchState {
    /// General-purpose registers.
    regs: VcpuRegs,
    /// Special registers and descriptor tables provided by userspace.
    sregs: VcpuSregs,
    /// VMX control-register state split into guest-visible and hardware values.
    control_regs: VcpuControlRegisters,
    /// Guest-visible MSRs emulated by the hypervisor.
    msrs: VcpuMsrs,
    /// FPU/SIMD context.
    fpu: FpuContext,
    /// CPUID entries visible to the guest.
    cpuid_entries: Vec<GuestCpuidEntry>,
    /// Whether userspace has provided CPUID entries with `KVM_SET_CPUID2`.
    cpuid_configured: bool,
}

impl GuestContext {
    /// Creates a guest vCPU context.
    ///
    /// The bootstrap vCPU, whose ID is zero, starts in the runnable state.
    /// Other vCPUs start in wait-for-SIPI state and become runnable after
    /// [`Self::receive_sipi`] accepts a startup vector.
    pub fn new(id: u32) -> Result<Self> {
        Ok(Self {
            id,
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

    /// Moves an AP vCPU from wait-for-SIPI state to runnable state.
    ///
    /// The startup vector is used to rebuild the vCPU's real-mode startup
    /// state. Calling this method for a vCPU that is not waiting for SIPI has
    /// no effect.
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

    /// Returns the guest general-purpose register state.
    pub fn regs(&self) -> VcpuRegs {
        self.arch.regs
    }

    /// Replaces the guest general-purpose register state.
    ///
    /// This method stores the values as guest-visible state. The caller is
    /// responsible for choosing register values that make sense for the guest
    /// execution mode and entry point.
    pub fn set_regs(&mut self, regs: VcpuRegs) {
        self.arch.regs = regs;
    }

    /// Returns the guest special-register state.
    ///
    /// The returned state contains the guest-visible control-register values,
    /// not the VMX-adjusted hardware values used internally for VM entry.
    pub fn sregs(&self) -> VcpuSregs {
        self.arch.sregs()
    }

    /// Replaces the guest special-register state.
    ///
    /// This method keeps derived context fields consistent with the supplied
    /// special registers. It updates VMX control-register shadows, synchronizes
    /// EFER state, mirrors FS/GS bases into the corresponding MSR state, and
    /// sanitizes the APIC base for this vCPU. The caller remains responsible
    /// for providing architecturally valid guest state.
    pub fn set_sregs(&mut self, mut sregs: VcpuSregs) {
        sregs.apic_base = sanitize_apic_base_for_vcpu(sregs.apic_base, self.id);
        self.arch.set_sregs(sregs);
    }

    /// Returns a guest general-purpose register by VMX register index.
    ///
    /// Invalid register indexes return zero.
    pub fn gpr(&self, index: u8) -> u64 {
        self.arch.gpr(index)
    }

    /// Updates a guest general-purpose register by VMX register index.
    ///
    /// The `width_byte` argument controls whether the low 1, 2, 4, or 8 bytes
    /// are updated. Invalid register indexes are ignored. The caller is
    /// responsible for using an index and width that match the emulated guest
    /// instruction.
    pub fn set_gpr(&mut self, index: u8, width_byte: u8, value: u64) {
        self.arch.set_gpr(index, width_byte, value);
    }

    /// Advances the guest instruction pointer.
    ///
    /// The caller is responsible for passing the length of the instruction
    /// that has actually been consumed or emulated.
    pub fn advance_rip(&mut self, len: u64) {
        self.arch.advance_rip(len);
    }

    /// Returns the guest instruction pointer.
    pub fn rip(&self) -> u64 {
        self.arch.rip()
    }

    /// Returns whether the guest vCPU is currently running.
    pub fn is_running(&self) -> bool {
        self.run == VcpuRunState::Running
    }

    /// Returns the guest-visible TSC value.
    pub fn guest_tsc(&self) -> u64 {
        use crate::arch::read_tsc;
        let tsc = read_tsc() as i64 + self.tsc_offset;
        if tsc < 0 {
            0
        } else {
            tsc as u64
        }
    }

    /// Returns the guest-visible value of a supported MSR.
    ///
    /// Unsupported MSR indexes return `None`.
    pub fn read_msr(&self, index: u32) -> Option<u64> {
        use x86::msr::*;

        match index {
            TSC => Some(self.guest_tsc()),
            IA32_BIOS_SIGN_ID => Some(0),
            _ => self.arch.try_msr(index),
        }
    }

    /// Sets the guest-visible value of a supported MSR.
    ///
    /// Returns `false` if the MSR index is not supported. Supported MSRs are
    /// stored in the context and may update derived state such as TSC offset,
    /// TSC deadline, APIC base, or EFER.LMA. The caller remains responsible
    /// for choosing MSR values that are meaningful for the guest.
    pub fn write_msr(&mut self, index: u32, value: u64) -> bool {
        use x86::msr::*;

        match index {
            TSC => {
                let raw_tsc = crate::arch::read_tsc();
                self.tsc_offset = value as i64 - raw_tsc as i64;
                true
            }
            IA32_TSC_ADJUST => {
                let old_value = self.arch.try_msr(IA32_TSC_ADJUST).unwrap_or(0);
                if !self.arch.set_msr(IA32_TSC_ADJUST, value) {
                    return false;
                }

                let delta = value as i64 - old_value as i64;
                self.tsc_offset += delta;
                true
            }
            IA32_APIC_BASE => {
                let apic_base = sanitize_apic_base_for_vcpu(value, self.id);
                self.arch.set_msr(IA32_APIC_BASE, apic_base)
            }
            IA32_EFER => {
                self.arch.set_efer(value);
                true
            }
            IA32_BIOS_SIGN_ID => true,
            IA32_TSC_DEADLINE => {
                if !self.arch.set_msr(IA32_TSC_DEADLINE, value) {
                    return false;
                }

                self.tsc_deadline = (value != 0).then_some(value);
                true
            }
            _ => self.arch.set_msr(index, value),
        }
    }

    // /// Updates the CPU topology visible to this guest vCPU.
    // ///
    // /// If CPUID has not been explicitly configured by the kernel, this method
    // /// refreshes the default CPUID entries to reflect the new topology.
    // pub fn set_guest_cpu_config(&mut self, config: GuestCpuConfig) {
    //     self.cpu_config = config;
    //     self.arch.refresh_cpuid_entries(config);
    // }

    /// Sets the CPUID entries visible to this vCPU.
    ///
    /// The caller remains responsible for choosing CPUID values that are
    /// meaningful for the guest.
    pub fn set_cpuid_entries(&mut self, entries: Vec<GuestCpuidEntry>) {
        self.arch.set_cpuid_entries(entries);
    }

    /// Returns the CPUID result visible to this vCPU.
    ///
    /// If no configured entry matches the requested function and index, this
    /// method returns a zeroed CPUID entry.
    pub fn cpuid_result(&self, function: u32, index: u32) -> GuestCpuidEntry {
        self.arch.cpuid_entry(function, index).unwrap_or_default()
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

    pub(crate) fn tsc_deadline(&self) -> Option<u64> {
        self.tsc_deadline
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
            cpuid_entries: default_cpuid_entries(GuestCpuConfig::default()),
            cpuid_configured: false,
        }
    }
}

impl Default for GuestContext {
    fn default() -> Self {
        Self::new(0).expect("failed to create guest context")
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum VcpuRunState {
    Running,
    #[default]
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

/// CPU topology visible to a guest vCPU.
#[derive(Clone, Copy, Debug, Default)]
pub struct GuestCpuConfig {
    /// The vCPU ID in the VM.
    pub vcpu_id: u32,
    /// The local APIC ID visible to the guest.
    pub lapic_id: u32,
    /// The number of vCPUs in the VM.
    pub vcpu_count: u32,
}

/// A CPUID result entry visible to a guest vCPU.
#[derive(Clone, Copy, Debug)]
pub struct GuestCpuidEntry {
    /// CPUID function, i.e., input `EAX`.
    pub function: u32,
    /// CPUID index/subleaf, i.e., input `ECX`.
    pub index: u32,
    /// KVM-compatible flags describing how the entry should be matched.
    pub flags: u32,
    /// Output `EAX`.
    pub eax: u32,
    /// Output `EBX`.
    pub ebx: u32,
    /// Output `ECX`.
    pub ecx: u32,
    /// Output `EDX`.
    pub edx: u32,
}

impl GuestCpuidEntry {
    fn new(function: u32, index: u32, flags: u32, result: CpuidResult) -> Self {
        Self {
            function,
            index,
            flags,
            eax: result.eax,
            ebx: result.ebx,
            ecx: result.ecx,
            edx: result.edx,
        }
    }

    fn matches(self, function: u32, index: u32) -> bool {
        if self.function != function {
            return false;
        }
        self.flags & GUEST_CPUID_FLAG_SIGNIFICANT_INDEX == 0 || self.index == index
    }
}

impl Default for GuestCpuidEntry {
    fn default() -> Self {
        Self::new(
            0,
            0,
            0,
            CpuidResult {
                eax: 0,
                ebx: 0,
                ecx: 0,
                edx: 0,
            },
        )
    }
}

/// Returns the default CPUID entries supported by the hypervisor.
pub fn default_cpuid_entries(cpu_config: GuestCpuConfig) -> Vec<GuestCpuidEntry> {
    const MAX_BASIC_CPUID: u32 = 0x16;
    const MAX_EXTENDED_CPUID: u32 = 0x8000_0008;

    let max_basic = cpuid(0, 0)
        .map(|result| result.eax)
        .unwrap_or(0)
        .max(MAX_BASIC_CPUID);
    let mut entries = Vec::new();

    for function in 0..=max_basic {
        match function {
            4 => push_cache_cpuid_entries(&mut entries, cpu_config),
            7 => push_indexed_cpuid_entry(&mut entries, function, 0, cpu_config),
            0x0b | 0x1f => push_topology_cpuid_entries(&mut entries, function, cpu_config),
            0x0d => push_indexed_cpuid_entry(&mut entries, function, 0, cpu_config),
            _ => entries.push(default_cpuid_entry(function, 0, 0, cpu_config)),
        }
    }

    let max_extended = cpuid(0x8000_0000, 0)
        .map(|result| result.eax)
        .unwrap_or(0x8000_0000)
        .min(MAX_EXTENDED_CPUID);
    for function in 0x8000_0000..=max_extended {
        entries.push(default_cpuid_entry(function, 0, 0, cpu_config));
    }

    entries
}

fn push_cache_cpuid_entries(entries: &mut Vec<GuestCpuidEntry>, cpu_config: GuestCpuConfig) {
    const MAX_CACHE_SUBLEAVES: u32 = 16;

    for index in 0..MAX_CACHE_SUBLEAVES {
        let entry = default_cpuid_entry(4, index, GUEST_CPUID_FLAG_SIGNIFICANT_INDEX, cpu_config);
        let cache_type = entry.eax & 0x1f;
        entries.push(entry);
        if cache_type == 0 {
            break;
        }
    }
}

fn push_topology_cpuid_entries(
    entries: &mut Vec<GuestCpuidEntry>,
    function: u32,
    cpu_config: GuestCpuConfig,
) {
    for index in 0..=2 {
        entries.push(default_cpuid_entry(
            function,
            index,
            GUEST_CPUID_FLAG_SIGNIFICANT_INDEX,
            cpu_config,
        ));
    }
}

fn push_indexed_cpuid_entry(
    entries: &mut Vec<GuestCpuidEntry>,
    function: u32,
    index: u32,
    cpu_config: GuestCpuConfig,
) {
    entries.push(default_cpuid_entry(
        function,
        index,
        GUEST_CPUID_FLAG_SIGNIFICANT_INDEX,
        cpu_config,
    ));
}

fn default_cpuid_entry(
    function: u32,
    index: u32,
    flags: u32,
    cpu_config: GuestCpuConfig,
) -> GuestCpuidEntry {
    let result = cpuid(function, index).unwrap_or(CpuidResult {
        eax: 0,
        ebx: 0,
        ecx: 0,
        edx: 0,
    });
    let result = sanitize_cpuid_result(function, index, result, cpu_config);

    GuestCpuidEntry::new(function, index, flags, result)
}

fn sanitize_cpuid_result(
    function: u32,
    index: u32,
    result: CpuidResult,
    cpu_config: GuestCpuConfig,
) -> CpuidResult {
    const CPUID_1_ECX_VMX: u32 = 1 << 5;
    const CPUID_1_ECX_FMA: u32 = 1 << 12;
    const CPUID_1_ECX_PCID: u32 = 1 << 17;
    const CPUID_1_ECX_X2APIC: u32 = 1 << 21;
    const CPUID_1_ECX_TSC_DEADLINE: u32 = 1 << 24;
    const CPUID_1_ECX_XSAVE: u32 = 1 << 26;
    const CPUID_1_ECX_OSXSAVE: u32 = 1 << 27;
    const CPUID_1_ECX_AVX: u32 = 1 << 28;
    const CPUID_1_EDX_APIC: u32 = 1 << 9;
    const CPUID_1_EDX_HTT: u32 = 1 << 28;
    const CPUID_7_EBX_FSGSBASE: u32 = 1 << 0;
    const CPUID_7_EBX_HLE: u32 = 1 << 4;
    const CPUID_7_EBX_AVX2: u32 = 1 << 5;
    const CPUID_7_EBX_INVPCID: u32 = 1 << 10;
    const CPUID_7_EBX_RTM: u32 = 1 << 11;
    const CPUID_7_EBX_AVX512F: u32 = 1 << 16;
    const CPUID_7_EBX_AVX512DQ: u32 = 1 << 17;
    const CPUID_7_EBX_AVX512CD: u32 = 1 << 28;
    const CPUID_7_EBX_AVX512BW: u32 = 1 << 30;
    const CPUID_7_EBX_AVX512VL: u32 = 1 << 31;
    const CPUID_7_ECX_AVX512VBMI: u32 = 1 << 1;
    const CPUID_7_ECX_VAES: u32 = 1 << 9;
    const CPUID_7_ECX_VPCLMULQDQ: u32 = 1 << 10;
    const CPUID_7_ECX_AVX512VNNI: u32 = 1 << 11;
    const CPUID_7_ECX_AVX512BITALG: u32 = 1 << 12;
    const CPUID_7_ECX_AVX512VPOPCNTDQ: u32 = 1 << 14;
    const CPUID_TSC_CRYSTAL_HZ: u32 = 1_000_000;

    let vcpu_count = cpu_config.vcpu_count.max(1);
    let apic_id = cpu_config.lapic_id;
    let CpuidResult {
        mut eax,
        mut ebx,
        mut ecx,
        mut edx,
    } = result;

    match function {
        0 => {
            eax = eax.max(0x16);
        }
        1 => {
            ecx &= !(CPUID_1_ECX_VMX
                | CPUID_1_ECX_FMA
                | CPUID_1_ECX_X2APIC
                | CPUID_1_ECX_TSC_DEADLINE
                | CPUID_1_ECX_PCID
                | CPUID_1_ECX_XSAVE
                | CPUID_1_ECX_OSXSAVE
                | CPUID_1_ECX_AVX);
            ebx = (ebx & 0x0000_ffff) | ((vcpu_count & 0xff) << 16) | ((apic_id & 0xff) << 24);
            edx |= CPUID_1_EDX_APIC;
            if vcpu_count > 1 {
                edx |= CPUID_1_EDX_HTT;
            } else {
                edx &= !CPUID_1_EDX_HTT;
            }
        }
        4 if (eax & 0x1f) != 0 => {
            let cores_per_package_minus_one = vcpu_count.saturating_sub(1).min(0x3f);
            eax = (eax & !(0x3f << 26)) | (cores_per_package_minus_one << 26);
        }
        7 if index == 0 => {
            ebx &= !(CPUID_7_EBX_FSGSBASE
                | CPUID_7_EBX_HLE
                | CPUID_7_EBX_AVX2
                | CPUID_7_EBX_RTM
                | CPUID_7_EBX_INVPCID
                | CPUID_7_EBX_AVX512F
                | CPUID_7_EBX_AVX512DQ
                | CPUID_7_EBX_AVX512CD
                | CPUID_7_EBX_AVX512BW
                | CPUID_7_EBX_AVX512VL);
            ecx &= !(CPUID_7_ECX_AVX512VBMI
                | CPUID_7_ECX_VAES
                | CPUID_7_ECX_VPCLMULQDQ
                | CPUID_7_ECX_AVX512VNNI
                | CPUID_7_ECX_AVX512BITALG
                | CPUID_7_ECX_AVX512VPOPCNTDQ);
        }
        0x0d => {
            eax = 0;
            ebx = 0;
            ecx = 0;
            edx = 0;
        }
        0x0b | 0x1f => {
            let topology = topology_cpuid(index, apic_id, vcpu_count);
            eax = topology.eax;
            ebx = topology.ebx;
            ecx = topology.ecx;
            edx = topology.edx;
        }
        0x15 => {
            if let Some(tsc_mhz) = virtual_tsc_mhz() {
                eax = 1;
                ebx = tsc_mhz;
                ecx = CPUID_TSC_CRYSTAL_HZ;
                edx = 0;
            }
        }
        0x16 => {
            if let Some(tsc_mhz) = virtual_tsc_mhz() {
                eax = tsc_mhz;
                ebx = tsc_mhz;
                ecx = 0;
                edx = 0;
            }
        }
        _ => {}
    }

    CpuidResult { eax, ebx, ecx, edx }
}

fn topology_cpuid(subleaf: u32, apic_id: u32, vcpu_count: u32) -> CpuidResult {
    if vcpu_count <= 1 {
        return CpuidResult {
            eax: 0,
            ebx: 0,
            ecx: subleaf,
            edx: apic_id,
        };
    }

    match subleaf {
        0 => CpuidResult {
            eax: 0,
            ebx: 1,
            ecx: 1 << 8,
            edx: apic_id,
        },
        1 => CpuidResult {
            eax: topology_apic_id_shift(vcpu_count),
            ebx: vcpu_count,
            ecx: (2 << 8) | 1,
            edx: apic_id,
        },
        _ => CpuidResult {
            eax: 0,
            ebx: 0,
            ecx: subleaf,
            edx: apic_id,
        },
    }
}

fn topology_apic_id_shift(vcpu_count: u32) -> u32 {
    u32::BITS - vcpu_count.saturating_sub(1).leading_zeros()
}

fn virtual_tsc_mhz() -> Option<u32> {
    let mhz = (tsc_freq().saturating_add(500_000)) / 1_000_000;
    u32::try_from(mhz).ok().filter(|&mhz| mhz != 0)
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

    fn set_cpuid_entries(&mut self, entries: Vec<GuestCpuidEntry>) {
        self.cpuid_entries = entries;
        self.cpuid_configured = true;
    }

    fn refresh_cpuid_entries(&mut self, cpu_config: GuestCpuConfig) {
        if !self.cpuid_configured {
            self.cpuid_entries = default_cpuid_entries(cpu_config);
        }
    }

    fn cpuid_entry(&self, function: u32, index: u32) -> Option<GuestCpuidEntry> {
        self.cpuid_entries
            .iter()
            .copied()
            .find(|entry| entry.matches(function, index))
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
        self.try_msr(index).unwrap_or_else(|| {
            error!("get unknown msr {:x}, return 0.", index);
            0
        })
    }

    fn try_msr(&self, index: u32) -> Option<u64> {
        use x86::msr::*;

        Some(match index {
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
            IA32_MISC_ENABLE => self.msrs.misc_enable,
            _ => return None,
        })
    }

    pub(crate) fn set_msr(&mut self, index: u32, value: u64) -> bool {
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
            IA32_MISC_ENABLE => self.msrs.misc_enable = value,
            _ => return false,
        }

        true
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
#[expect(
    missing_docs,
    reason = "KVM-compatible register field names are self-describing."
)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
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
pub(crate) struct VcpuMsrs {
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
    pub misc_enable: u64,
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
            misc_enable: 0,
        }
    }
}

/// Guest special register state.
#[expect(
    missing_docs,
    reason = "KVM-compatible register field names are self-describing."
)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
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
#[expect(
    missing_docs,
    reason = "KVM-compatible segment field names are self-describing."
)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
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
#[expect(
    missing_docs,
    reason = "KVM-compatible descriptor-table field names are self-describing."
)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
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
            idt: VcpuDtable {
                base: 0,
                limit: 0x03ff,
                padding: [0; 3],
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
