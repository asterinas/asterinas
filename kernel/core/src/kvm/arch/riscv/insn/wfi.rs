// SPDX-License-Identifier: MPL-2.0

use super::{InsnFunc, KvmInsnReturn};

const INSN_MASK_WFI: usize = 0xffffffff;
const INSN_MATCH_WFI: usize = 0x10500073;

pub(super) const WFI: InsnFunc = InsnFunc {
    mask: INSN_MASK_WFI,
	should_match: INSN_MATCH_WFI,
	func: &wfi_insn,
};

fn wfi_insn(_insn: usize) -> KvmInsnReturn {
    ostd::task::Task::yield_now();
    KvmInsnReturn::ContinueNextSepc
}
