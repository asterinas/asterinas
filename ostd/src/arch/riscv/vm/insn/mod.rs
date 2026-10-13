/// 1

mod csr;
mod wfi;
mod wrs;

use wrs::WRS;
use wfi::WFI;
use csr::*;

/// Return values used by function emulating a particular instruction
pub enum KvmInsnReturn {
	/// 1
	KvmInsnExitToUserSpace = 0,
	/// 2
	KvmInsnContinueNextSepc,
	/// 3
	KvmInsnContinueSameSepc,
	/// 4
	KvmInsnIllegalTrap,
	/// 5
	KvmInsnVirtualTrap
}

type InsnFunction = dyn Fn(usize) -> KvmInsnReturn + Sync + Send + 'static;

pub(super) struct InsnFunc {
	pub(super) mask: usize,
	pub(super) should_match: usize,
	/*
	 * Possible return values are as follows:
	 * 1) Returns < 0 for error case
	 * 2) Returns 0 for exit to user-space
	 * 3) Returns 1 to continue with next sepc
	 * 4) Returns 2 to continue with same sepc
	 * 5) Returns 3 to inject illegal instruction trap and continue
	 * 6) Returns 4 to inject virtual instruction trap and continue
	 *
	 * Use enum kvm_insn_return for return values
	 */
	pub(super) func: &'static InsnFunction,
}

/// 1
const SYSTEM_OPCODE_FUNCS: [InsnFunc; 8] = [
    CSRRW,
    CSRRS,
	CSRRC,
    CSRRWI,
	CSRRSI,
    CSRRCI,
	WFI,
    WRS,
];

/// 1
pub fn insn_len(insn: usize) -> usize {
	if (insn & 0x3) != 0x3 {
        return 2;
    } else {
        return 4;
    }
}

/// 1
pub fn handle_system_opcode_insn(insn: usize) -> KvmInsnReturn {
	let mut ret = KvmInsnReturn::KvmInsnExitToUserSpace;
    for i in 0..8 {
        let func = &SYSTEM_OPCODE_FUNCS[i];
        if insn & func.mask == func.should_match {
            let func = func.func;
            ret = func(insn);
        }
    }
    ret
}
