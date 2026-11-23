// SPDX-License-Identifier: MPL-2.0

/// The shift of `MODE` in HGATP.
pub const HGATP_MODE_SHIFT: usize = 60;
pub(super) const HGATP_MODE_LEVEL_48: usize = 0x9;
/// The Sv48*4 MODE for HGATP.
pub const HGATP_MODE_48: usize = HGATP_MODE_LEVEL_48 << HGATP_MODE_SHIFT;
pub(super) const HGATP_PPN: usize = 0xFFFFFFFFFFF;

/// A flag indicating whether an interruption has occurred(from host).
pub const CAUSE_IRQ_FLAG: usize = 0x1 << 63;
/// The irq mask of SIP.
pub const IRQ_LOCAL_MASK: usize = 0x3FFF;

/// The shift from VSIP to HVIP.
pub const VSIP_TO_HVIP_SHIFT: usize =
    InterruptCauses::VsSoft as usize - InterruptCauses::SSoft as usize;
/// A mask for VSIP that retains only the valid bits
pub const VSIP_VALID_MASK: usize = (1 << InterruptCauses::SSoft as usize)
    | (1 << InterruptCauses::STimer as usize)
    | (1 << InterruptCauses::SExt as usize)
    | (1 << InterruptCauses::PmuOvf as usize);

pub(super) const KVM_HEDELEG_DEFAULT: usize = 0xB30D;
pub(super) const KVM_HIDELEG_DEFAULT: usize = 0x444;
pub(super) enum InterruptCauses {
    /// Supervisor software interrupt.
    SSoft = 1,
    /// Virtual Supervisor software interrupt.
    VsSoft = 2,
    /// Machine software interrupt.
    MSoft = 3,
    /// Supervisor timer interrupt.
    STimer = 5,
    /// Virtual Supervisor timer interrupt.
    VsTimer = 6,
    /// Machine timer interrupt.
    MTimer = 7,
    /// Supervisor external interrupt.
    SExt = 9,
    /// Virtual Supervisor external interrupt.
    VsExt = 10,
    /// Machine external interrupt.
    MExt = 11,
    /// Supervisor guest external interrupt.
    SGext = 12,
    /// Performance Monitoring Unit overflow.
    PmuOvf = 13,
}
