use crate::{arch::vm::vmx::*, prelude::*};

/// Interruptibility-state blocking bits (STI / MOV-SS blocking).
const BLOCKING_BY_STI: u32 = 1 << 0;
const BLOCKING_BY_MOV_SS: u32 = 1 << 1;
const RFLAGS_IF: usize = 1 << 9;
pub(crate) const INTR_INFO_VALID_MASK: u32 = 0x8000_0000;
pub(crate) const INTR_TYPE_EXT_INTR: u32 = 0;

/// remove BLOCKING_BY_STI bit in the guest interruptibility state(VMCS)
pub fn clear_block_by_sti() -> Result<()> {
    let interruptibility = VmcsGuest32::INTERRUPTIBILITY_STATE.read()?;
    VmcsGuest32::INTERRUPTIBILITY_STATE.write(interruptibility & !BLOCKING_BY_STI)
}

/// Check whether the guest is currently in a state where an external interrupt
/// can be injected (RFLAGS.IF == 1 and no blocking-by-STI/MOV-SS).
pub(crate) fn vmx_interrupt_injectable() -> Result<bool> {
    let rflags = VmcsGuestNW::RFLAGS.read()?;
    let interruptibility = VmcsGuest32::INTERRUPTIBILITY_STATE.read()?;
    let if_set = (rflags & RFLAGS_IF) != 0;
    let not_blocking = (interruptibility & (BLOCKING_BY_STI | BLOCKING_BY_MOV_SS)) == 0;
    Ok(if_set && not_blocking)
}

/// Enable interrupt-window exiting in the primary processor-based controls.
pub(crate) fn enable_interrupt_window_exiting() -> Result<()> {
    let cur = VmcsControl32::PRIMARY_PROCBASED_EXEC_CONTROLS.read()?;
    VmcsControl32::PRIMARY_PROCBASED_EXEC_CONTROLS.write(cur | (1 << 2))?;
    Ok(())
}

/// Disable interrupt-window exiting in the primary processor-based controls.
pub(crate) fn disable_interrupt_window_exiting() -> Result<()> {
    let cur = VmcsControl32::PRIMARY_PROCBASED_EXEC_CONTROLS.read()?;
    VmcsControl32::PRIMARY_PROCBASED_EXEC_CONTROLS.write(cur & !(1 << 2))?;
    Ok(())
}
