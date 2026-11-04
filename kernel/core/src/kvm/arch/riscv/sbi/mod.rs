// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;
use crate::kvm::arch::vcpu::VcpuArch;
use ostd::arch::vm::GuestExitInfo;

mod base;

use base::VCPU_SBI_EXT_BASE;

/// The status of SBI extensions
#[derive(Clone, Copy, PartialEq)]
enum KvmRiscvSbiExtStatus {
	Uninitialized,
	Unavailable,
	Enabled,
	Disabled,
}

/// SBI extension IDs specific to KVM. 
/// This is not the same as the SBI extension IDs defined by the RISC-V SBI specification.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum KvmRiscvSbiExtId {
	V01,
	Time,
	Ipi,
	Rfence,
	Srst,
	Hsm,
	Pmu,
	Experimental,
	Vendor,
	Dbcn,
	Sta,
	Susp,
	Fwft,
	Mpxy,
	Max,
}

/// The Sbi context of Vcpu
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VcpuSbiContext {
	return_handled: usize,
	ext_status: [KvmRiscvSbiExtStatus; KvmRiscvSbiExtId::Max as usize + 1],
    shmem: u64,
	last_steal:u64,
}

impl VcpuSbiContext {
    /// Create a new SbiContext
    pub(super) fn new() -> Self {
        Self {
            return_handled: 0,
            ext_status: [KvmRiscvSbiExtStatus::Uninitialized; KvmRiscvSbiExtId::Max as usize + 1],
            shmem: 0,
            last_steal: 0,
        }
    }

    /// Init the SbiContext
    pub(super) fn init(&mut self) -> Result<()> {
        for &(ext_id, ext) in SBI_EXT_TABLE.iter() {
            let idx = ext_id as usize;
            self.ext_status[idx] = if ext.default_disabled {
                KvmRiscvSbiExtStatus::Disabled
            } else {
                KvmRiscvSbiExtStatus::Enabled
            };
            if let Some(func) = ext.init_fn {
                func(self)?;
            }
        }
        Ok(())
    }
}

/// The return value of SbiHandler
#[derive(Clone, Copy, Debug, Default)]
struct KvmVcpuSbiReturn {
	pub out_val: usize,
	pub err_val: usize,
	pub utrap: GuestExitInfo,
	pub uexit: bool,
}

enum KvmVcpuSbiReturnError {
    Failure	= -1,
    NotSupported = -2,
    InvalidParam = -3,
    Denied = -4,
    InvalidAddress	= -5,
    AlreadyAvailable = -6,
    AlreadyStarted = -7,
    AlreadyStopped = -8,
    NoShmem = -9,
    InvalidState = -10,
    BadRange = -11,
    Timeout = -12,
    Io = -13,
    DeniedLocked = -14,
}

type SbiHandlerFunction = dyn Fn(&VcpuArch) -> Result<KvmVcpuSbiReturn> + Sync + Send + 'static;
type SbiProbeFunction = dyn Fn() -> Result<KvmVcpuSbiReturn> + Sync + Send + 'static;
type SbiInitFunction = dyn Fn(&mut VcpuSbiContext) -> Result<usize> + Sync + Send + 'static;
type SbiDeinitFunction = dyn Fn() -> Result<KvmVcpuSbiReturn> + Sync + Send + 'static;
type SbiResetFunction = dyn Fn() -> Result<usize> + Sync + Send + 'static;
type SbiGetStateRegIdFunction = dyn Fn(usize) -> Result<usize> + Sync + Send + 'static;
type SbiGetStateRegCountFunction = dyn Fn() -> usize + Sync + Send + 'static;
type SbiGetStateRegFunction = dyn Fn() -> Result<KvmVcpuSbiReturn> + Sync + Send + 'static;
type SbiSetStateRegFunction = dyn Fn() -> Result<KvmVcpuSbiReturn> + Sync + Send + 'static;

struct KvmVcpuSbiExtension {
	extid_start: usize,
	extid_end: usize,

	default_disabled: bool,

	/// SBI extension handler. 
    /// It can be defined for a given extension or group of extension.
	/// But it should always return linux error codes rather than SBI specific error codes.
	handler_fn: &'static SbiHandlerFunction,

    /// Extension specific probe function
    probe_fn: Option<&'static SbiProbeFunction>,

	/// Init/deinit function called once during VCPU init/destroy. 
	/// These might be use if the SBI extensions need to allocate or do specific init time only configuration.
	init_fn: Option<&'static SbiInitFunction>,
    deinit_fn: Option<&'static SbiDeinitFunction>,

    reset_fn: Option<&'static SbiResetFunction>,

    state_reg_subtype: usize,
    get_state_reg_count_fn: Option<&'static SbiGetStateRegCountFunction>,
	get_state_reg_id_fn: Option<&'static SbiGetStateRegIdFunction>,
    get_state_reg_fn: Option<&'static SbiGetStateRegFunction>,
    set_state_reg_fn: Option<&'static SbiSetStateRegFunction>,
}

impl KvmVcpuSbiExtension {
    /// Create an SBI extension with default optional fields.
    ///
    /// `default_disabled`, `probe`, `init`, `deinit`, `reset`, `state_reg_subtype`,
    /// and all state-reg hooks default to disabled/zero; 
    /// use struct update syntax to override only the fields an extension actually needs:
    ///
    /// ```ignore
    /// KvmVcpuSbiExtension {
    ///     default_disabled: true,
    ///     init: Some(&sbi_ext_x_init),
    ///     ..KvmVcpuSbiExtension::new(SBI_EXT_X, SBI_EXT_X, &sbi_ext_x_handler)
    /// }
    /// ```
    pub(super) const fn new(
        extid_start: usize,
        extid_end: usize,
        handler_fn: &'static SbiHandlerFunction,
    ) -> Self {
        Self {
            extid_start,
            extid_end,
            default_disabled: false,
            handler_fn,
            probe_fn: None,
            init_fn: None,
            deinit_fn: None,
            reset_fn: None,
            state_reg_subtype: 0,
            get_state_reg_count_fn: None,
            get_state_reg_id_fn: None,
            get_state_reg_fn: None,
            set_state_reg_fn: None,
        }
    }
}

// How do I add a new SBI_EXT simulation?
// 
// exts not yet added: All of `KvmRiscvSbiExtId` except `Max`
// exts that have been added but not yet fully implemented: BASE
//
// First, you need to add [fwft.rs/ipi.rs/....] in the current directory
// Then, declare their KvmVcpuSbiExtension objects.
// Then add it to `SBI_EXT_TABLE` to facilitate lookups.
//
// You can refer to the EXT_BASE(base.rs) for implementation.
//
// The expected size when fully implemented is 15, now is 1
const SBI_EXT_TABLE_SIZE: usize = 1;

/// `SBI_EXT_TABLE`: each entry pairs an extension with its `KvmRiscvSbiExtId` slot.
///  `VCPU_SBI_EXT_BASE` intentionally maps to `KvmRiscvSbiExtId::Max` (the sentinel slot),
///  so BASE does not occupy a regular `ext_status` slot.
const SBI_EXT_TABLE: [(KvmRiscvSbiExtId, &KvmVcpuSbiExtension); SBI_EXT_TABLE_SIZE] = [
    (KvmRiscvSbiExtId::Max,          &VCPU_SBI_EXT_BASE),
];

fn vcpu_sbi_find_ext(
    arch: &VcpuArch,
    idx: usize,
) -> Result<&'static KvmVcpuSbiExtension> {
    for (id, ext) in SBI_EXT_TABLE.iter() {
        if !(ext.extid_start..=ext.extid_end).contains(&idx) {
            continue;
        }

        match *id {
            // Individual control of the enabled status per vCPU (such as for basic extensions) is not supported;
            // therefore, access is granted automatically as long as the IDs match.
            x if x >= KvmRiscvSbiExtId::Max => return Ok(ext),

            // This extension supports control on a per-vCPU basis;
            // its application is determined  by whether it has been explicitly enabled for the current vCPU.
            other => {
                if arch.sbi_context.lock().ext_status[other as usize] == KvmRiscvSbiExtStatus::Enabled {
                    return Ok(ext);
                }
                return Err(Error::new(Errno::ENOENT));
            }
        }
    }

    Err(Error::new(Errno::ENOENT))
}

pub(super) fn reset_sbi(inner: &VcpuArch) -> Result<()> {
    for &(ext_id, ext) in SBI_EXT_TABLE.iter() {
        if let Some(func) = ext.reset_fn {
            if inner.sbi_context.lock().ext_status[ext_id as usize] != KvmRiscvSbiExtStatus::Enabled {
                func()?;
            }
        }
    }
    Ok(())
}

/// Handle vm's sbi_ecall
pub fn sbi_ecall(inner: &VcpuArch) -> Result<usize> {
    let ext = vcpu_sbi_find_ext(inner,inner.inner.lock().guest_context.a7)?;
    let func = ext.handler_fn;

    let rt = func(inner)?;

    if !rt.uexit {
        inner.inner.lock().guest_context.a0 = rt.err_val;
    }

    inner.inner.lock().guest_context.sepc+=4;
    inner.inner.lock().guest_context.a1 = rt.out_val;

    
    Ok(1)
}
