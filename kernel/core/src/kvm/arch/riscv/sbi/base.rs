// SPDX-License-Identifier: MPL-2.0

use super::{KvmVcpuSbiExtension, KvmVcpuSbiReturn, KvmVcpuSbiReturnError};
use crate::kvm::arch::vcpu::VcpuArch;
use crate::prelude::*;

const SBI_EXT_BASE: usize = 0x10;

const SBI_EXT_BASE_GET_SPEC_VERSION: usize = 0;
const SBI_EXT_BASE_GET_IMP_ID: usize = 1;
const SBI_EXT_BASE_GET_IMP_VERSION: usize = 2;
const SBI_EXT_BASE_PROBE_EXT: usize = 3;
const SBI_EXT_BASE_GET_MVENDORID: usize = 4;
const SBI_EXT_BASE_GET_MARCHID: usize = 5;
const SBI_EXT_BASE_GET_MIMPID: usize = 6;

// SBI version
const KVM_SBI_VERSION_MAJOR: usize = 3;
const KVM_SBI_SPEC_VERSION_MAJOR_SHIFT: usize = 24;
const KVM_SBI_VERSION_MINOR: usize = 0;

// The implementer's identifier
// For now, let's make the VM believe that we are Linux.
// Later on, we can change it to our own version.
const LINUX_VERSION_CODE: usize = 0x70200;
const KVM_SBI_IMP_ID: usize = 3;
const KVM_SBI_IMP_VERSION: usize = LINUX_VERSION_CODE;

pub(super) const VCPU_SBI_EXT_BASE: KvmVcpuSbiExtension =
    KvmVcpuSbiExtension::new(SBI_EXT_BASE, SBI_EXT_BASE, &sbi_ext_base_handler);

fn sbi_ext_base_handler(arch: &VcpuArch) -> Result<KvmVcpuSbiReturn> {
    let mut rt = KvmVcpuSbiReturn::default();

    let a6 = arch.inner.lock().guest_context.a6;
    match a6 {
        SBI_EXT_BASE_GET_SPEC_VERSION => rt.out_val = (KVM_SBI_VERSION_MAJOR << KVM_SBI_SPEC_VERSION_MAJOR_SHIFT) | KVM_SBI_VERSION_MINOR,
        SBI_EXT_BASE_GET_IMP_ID => rt.out_val = KVM_SBI_IMP_ID,
        SBI_EXT_BASE_GET_IMP_VERSION => rt.out_val = KVM_SBI_IMP_VERSION,
        SBI_EXT_BASE_PROBE_EXT => rt.out_val = 0,
        SBI_EXT_BASE_GET_MVENDORID => rt.out_val = arch.inner.lock().mvendorid,
        SBI_EXT_BASE_GET_MARCHID => rt.out_val = arch.inner.lock().marchid,
        SBI_EXT_BASE_GET_MIMPID => rt.out_val = arch.inner.lock().mimpid,
        _ => rt.err_val = KvmVcpuSbiReturnError::NotSupported as usize,
    }

    Ok(rt)
}
