///     
/// mod vm;
///     mod ept;
///     mod x86;
///     mod vmx;
///     mod vmcs;
///     mod context; // implements GuestContext
///     mod exit;
///         mod handler;
///     mod emulate;
///         mod cr;
///         mod cpuid;
///         mod msr;
///     
/// mod vm;  // implements GuestMode
///          // new
///          // execute
///     mod gpm_space; // implements GuestPhysMemSpace
///
///  
pub(crate) mod context;
mod emulate;
pub(crate) mod ept;
pub(crate) mod exit;
pub(crate) mod interrupt;
pub(crate) mod vmcs;
pub(crate) mod vmx;
pub(crate) mod x86;

pub use self::{
    context::{GuestContext, GuestCpuConfig, VcpuDtable, VcpuRegs, VcpuSegment, VcpuSregs},
    exit::GuestExitInfo,
    vmx::{VmxExitReason, init_vmx},
};
