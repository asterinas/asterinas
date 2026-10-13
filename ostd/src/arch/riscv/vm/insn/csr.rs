use super::{InsnFunc, KvmInsnReturn};

const INSN_MATCH_CSRRW: usize = 0x1073;
const INSN_MASK_CSRRW: usize = 0x707f;
const INSN_MATCH_CSRRS: usize = 0x2073;
const INSN_MASK_CSRRS: usize = 0x707f;
const INSN_MATCH_CSRRC: usize = 0x3073;
const INSN_MASK_CSRRC: usize = 0x707f;
const INSN_MATCH_CSRRWI: usize = 0x5073;
const INSN_MASK_CSRRWI: usize = 0x707f;
const INSN_MATCH_CSRRSI: usize = 0x6073;
const INSN_MASK_CSRRSI: usize = 0x707f;
const INSN_MATCH_CSRRCI: usize = 0x7073;
const INSN_MASK_CSRRCI: usize = 0x707f;

pub(super) const CSRRW: InsnFunc = InsnFunc {
    mask: INSN_MASK_CSRRW,
	should_match: INSN_MATCH_CSRRW,
	func: &csr_insn,
};

pub(super) const CSRRS: InsnFunc = InsnFunc {
    mask: INSN_MASK_CSRRS,
	should_match: INSN_MATCH_CSRRS,
	func: &csr_insn,
};

pub(super) const CSRRC: InsnFunc = InsnFunc {
    mask: INSN_MASK_CSRRC,
	should_match: INSN_MATCH_CSRRC,
	func: &csr_insn,
};

pub(super) const CSRRWI: InsnFunc = InsnFunc {
    mask: INSN_MASK_CSRRWI,
	should_match: INSN_MATCH_CSRRWI,
	func: &csr_insn,
};

pub(super) const CSRRSI: InsnFunc = InsnFunc {
    mask: INSN_MASK_CSRRSI,
	should_match: INSN_MATCH_CSRRSI,
	func: &csr_insn,
};

pub(super) const CSRRCI: InsnFunc = InsnFunc {
    mask: INSN_MASK_CSRRCI,
	should_match: INSN_MATCH_CSRRCI,
	func: &csr_insn,
};

fn csr_insn(_insn: usize) -> KvmInsnReturn {
    todo!("CSR insn emul is not impled!")
}
