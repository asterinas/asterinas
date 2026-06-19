use crate::{arch::vm::vmx::VmxExitInfo, mm::Gpaddr};

mod handler;

pub use handler::vmexit_handler;

pub struct GuestExitInfo {
    pub exit_reason: u32,
    pub instruction_len: u32,
    pub exit_qualification: u64,
    pub guest_phys_addr: Gpaddr,
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
