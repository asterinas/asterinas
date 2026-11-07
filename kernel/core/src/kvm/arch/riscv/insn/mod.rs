// SPDX-License-Identifier: MPL-2.0

mod wfi;

use wfi::WFI;

/// The shift of insns
pub const INSN_OPCODE_SHIFT: usize = 2;
/// The mask of insns
pub const INSN_OPCODE_MASK: usize = 0x007c;

/// Return values used by function emulating a particular instruction
pub enum KvmInsnReturn {
	/// Need exit to user-space
	ExitToUserSpace = 0,
	/// Continue with next sepc
	ContinueNextSepc,
	/// Continue with same sepc
	ContinueSameSepc,
	/// Inject illegal instruction trap and continue
	IllegalTrap,
	/// Inject virtual instruction trap and continue
	VirtualTrap
}

type InsnFunction = dyn Fn(usize) -> KvmInsnReturn + Sync + Send + 'static;

struct InsnFunc {
	mask: usize,
	should_match: usize,
	func: &'static InsnFunction,
}

// How do I add a new instruction simulation?
// 
// Insns not yet added: CSRRW, CSRRS, CSRRC, CSRRWI, CSRRSI, CSRRCI, WRS
// Insns that have been added but not yet fully implemented: WFI
//
// First, you need to add [csr.rs/wrs.rs] in the current directory
// Then, declare their InsnFunc objects.
// Then add it to `SYSTEM_OPCODE_FUNCS` to facilitate lookups.
//
// You can refer to the WFI instruction for implementation.
//
// The expected size when fully implemented is 8
const SYSTEM_OPCODE_FUNCS_SIZE:usize  = 1;

const SYSTEM_OPCODE_FUNCS: [InsnFunc; SYSTEM_OPCODE_FUNCS_SIZE] = [
	WFI,
];

/// Get the length of the insn
pub fn insn_len(insn: usize) -> usize {
	if (insn & 0x3) != 0x3 {
        return 2;
    } else {
        return 4;
    }
}

/// Handle the system insns
pub fn handle_system_opcode_insn(insn: usize) -> KvmInsnReturn {
	let mut ret = KvmInsnReturn::ExitToUserSpace;
    for i in 0..SYSTEM_OPCODE_FUNCS_SIZE {
        let func = &SYSTEM_OPCODE_FUNCS[i];
        if insn & func.mask == func.should_match {
            let func = func.func;
            ret = func(insn);
        }
    }
    ret
}
