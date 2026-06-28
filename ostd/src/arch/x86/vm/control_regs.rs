// SPDX-License-Identifier: MPL-2.0

//! Provides VMX control-register shadows for guest-visible `CR0` and `CR4`.

use x86_64::registers::control::{Cr0Flags, Cr4Flags};

use super::{types::VcpuSregs, vmx::Msr};

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
    pub(crate) fn from_sregs(sregs: &VcpuSregs) -> Self {
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

    pub(crate) fn write_cr0(&mut self, guest_value: u64) {
        self.cr0 = VcpuControlRegister::for_cr0_guest_value(guest_value);
    }

    pub(crate) fn write_cr4(&mut self, guest_value: u64) {
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

    pub(crate) fn host_mask(&self) -> u64 {
        self.host_mask
    }

    pub(crate) fn read_shadow(&self) -> u64 {
        self.read_shadow
    }

    pub(crate) fn real(&self) -> u64 {
        self.real
    }

    pub(crate) fn guest_value(&self) -> u64 {
        (self.real & !self.host_mask) | (self.read_shadow & self.host_mask)
    }

    fn for_cr0_guest_value(guest_value: u64) -> Self {
        Self::from_vmcs(cr0_host_mask(), guest_value, cr0_real_value(guest_value))
    }

    fn for_cr4_guest_value(guest_value: u64) -> Self {
        Self::from_vmcs(cr4_host_mask(), guest_value, cr4_real_value(guest_value))
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
