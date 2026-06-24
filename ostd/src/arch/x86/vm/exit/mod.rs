use crate::{arch::vm::vmx::VmxExitInfo, mm::Gpaddr};

mod handler;

pub use handler::vmexit_handler;

/// Describes a VM exit that should be handled outside OSTD.
pub struct GuestExitInfo {
    /// The VMX exit reason.
    pub exit_reason: u32,
    /// The length of the instruction that caused the exit.
    pub instruction_len: u32,
    /// VMX exit qualification.
    pub exit_qualification: u64,
    /// Guest physical address associated with the exit, if any.
    pub guest_phys_addr: Gpaddr,
    /// Guest instruction pointer at the exit.
    pub guest_rip: Gpaddr,
}

impl From<VmxExitInfo> for GuestExitInfo {
    fn from(info: VmxExitInfo) -> Self {
        GuestExitInfo {
            exit_reason: info.exit_reason,
            instruction_len: info.instruction_len,
            exit_qualification: info.exit_qualification,
            guest_phys_addr: info.guest_phys_addr,
            guest_rip: info.guest_rip,
        }
    }
}
