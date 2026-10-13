use super::{InsnFunc, KvmInsnReturn};

const INSN_MASK_WRS: usize = 0xffffffff;
const INSN_MATCH_WRS: usize = 0x00d00073;

pub(super) const WRS: InsnFunc = InsnFunc {
    mask: INSN_MASK_WRS,
	should_match: INSN_MATCH_WRS,
	func: &wrs_insn,
};

fn wrs_insn(_insn: usize) -> KvmInsnReturn {
    todo!("WRS insn emul is not impled!")
}
