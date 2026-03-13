use alloc::{boxed::Box, sync::Arc};

use num_enum::TryFromPrimitiveError;

use crate::{
    mm::Vaddr,
    task::seccomp::cbpf_opcodes::{ClassicBpfOpcode, SeccompOpcode},
};

#[expect(non_camel_case_types)]

pub enum SeccompOp {
    SECCOMP_SET_MODE_STRICT,
    SECCOMP_SET_MODE_FILTER,
    // SECCOMP_GET_ACTION_AVAIL,
    // SECCOMP_GET_NOTIF_SIZES
}

#[expect(non_camel_case_types)]
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SeccompMode {
    SECCOMP_MODE_DISABLED,
    SECCOMP_MODE_STRICT,
    SECCOMP_MODE_FILTER,
}

#[derive(Debug, Clone, Copy, Pod)]
pub struct UserspaceFilterMeta {
    pub user_buf_ptr: Vaddr,
    pub user_buf_len: usize,
}

#[derive(Debug, Clone)]
pub struct SeccompTask {
    pub mode: SeccompMode,
    pub leaf_filter: Option<Arc<SeccompFilterProg>>,
}

#[derive(Debug, Clone)]
pub struct SeccompFilterProg {
    pub ins: Box<[SeccompFilterBlock]>,
    pub prev: Option<Arc<SeccompFilterProg>>,
    // missing cache, log, notif related stuff, possibly others
}

#[derive(Debug, Clone, Copy, Pod)]
pub struct RawFilterBlock {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

impl TryInto<FilterBlock> for RawFilterBlock {
    type Error = TryFromPrimitiveError<ClassicBpfOpcode>;

    fn try_into(self) -> Result<FilterBlock, Self::Error> {
        let code = ClassicBpfOpcode::try_from(self.code);

        match code {
            Ok(code) => Ok(FilterBlock {
                code,
                jt: self.jt,
                jf: self.jf,
                k: self.k,
            }),
            Err(e) => Err(e),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FilterBlock {
    pub code: ClassicBpfOpcode,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

#[derive(Debug, Clone)]
pub struct SeccompFilterBlock {
    pub code: SeccompOpcode,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

impl SeccompTask {
    pub fn new() -> Self {
        SeccompTask {
            mode: SeccompMode::SECCOMP_MODE_DISABLED,
            leaf_filter: None,
        }
    }
}

pub mod cbpf_opcodes {

    use num_enum::TryFromPrimitive;

    // class
    pub const BPF_LD: u16 = 0x00;
    pub const BPF_LDX: u16 = 0x01;
    pub const BPF_ST: u16 = 0x02;
    pub const BPF_STX: u16 = 0x03;
    pub const BPF_ALU: u16 = 0x04;
    pub const BPF_JMP: u16 = 0x05;
    pub const BPF_RET: u16 = 0x06;
    pub const BPF_MISC: u16 = 0x07;

    // size
    pub const BPF_W: u16 = 0x00;
    pub const BPF_H: u16 = 0x08;
    pub const BPF_B: u16 = 0x10;

    // mode
    pub const BPF_IMM: u16 = 0x00;
    pub const BPF_ABS: u16 = 0x20;
    pub const BPF_IND: u16 = 0x40;
    pub const BPF_MEM: u16 = 0x60;
    pub const BPF_LEN: u16 = 0x80;
    pub const BPF_MSH: u16 = 0xa0;

    // op
    pub const BPF_ADD: u16 = 0x00;
    pub const BPF_SUB: u16 = 0x10;
    pub const BPF_MUL: u16 = 0x20;
    pub const BPF_DIV: u16 = 0x30;
    pub const BPF_OR: u16 = 0x40;
    pub const BPF_AND: u16 = 0x50;
    pub const BPF_LSH: u16 = 0x60;
    pub const BPF_RSH: u16 = 0x70;
    pub const BPF_NEG: u16 = 0x80;
    pub const BPF_MOD: u16 = 0x90;
    pub const BPF_XOR: u16 = 0xa0;

    pub const BPF_JA: u16 = 0x00;
    pub const BPF_JEQ: u16 = 0x10;
    pub const BPF_JGT: u16 = 0x20;
    pub const BPF_JGE: u16 = 0x30;
    pub const BPF_JSET: u16 = 0x40;

    // src
    pub const BPF_K: u16 = 0x00;
    pub const BPF_X: u16 = 0x08;

    // ret
    pub const BPF_A: u16 = 0x10;

    // misc
    pub const BPF_TAX: u16 = 0x00;
    pub const BPF_TXA: u16 = 0x80;

    pub const BPF_MAXINS: usize = 4096;
    pub const BPF_MEMWORDS: u32 = 16;

    // Linux uses -0x1000 literal as it's an offset used for referencing ancillary data.
    // TODO check whether u32 is appropriate or a smaller uint should be used instead
    pub const SKF_AD_OFF: u32 = 4096;

    #[repr(u32)]
    #[derive(TryFromPrimitive)]
    #[expect(non_camel_case_types)]
    pub enum AncOps {
        SKF_AD_PROTOCOL = 0,
        SKF_AD_PKTTYPE = 4,
        SKF_AD_IFINDEX = 8,
        SKF_AD_NLATTR = 12,
        SKF_AD_NLATTR_NEST = 16,
        SKF_AD_MARK = 20,
        SKF_AD_QUEUE = 24,
        SKF_AD_HATYPE = 28,
        SKF_AD_RXHASH = 32,
        SKF_AD_CPU = 36,
        SKF_AD_ALU_XOR_X = 40,
        SKF_AD_VLAN_TAG = 44,
        SKF_AD_VLAN_TAG_PRESENT = 48,
        SKF_AD_PAY_OFFSET = 52,
        SKF_AD_RANDOM = 56,
        SKF_AD_VLAN_TPID = 60,
    }

    pub const SKF_AD_MAX: u32 = 64;
    pub const SKF_NET_OFF: u32 = 1048576; // TODO: look at SKF_AD_OFF. -0x100000 in Linux
    pub const SKF_LL_OFF: u32 = 2097152; // -0x200000 in Linux
    pub const BPF_NET_OFF: u32 = SKF_NET_OFF;
    pub const BPF_LL_OFF: u32 = SKF_LL_OFF;

    #[repr(u16)]
    #[derive(Debug, Clone, Copy, TryFromPrimitive)]
    #[expect(non_camel_case_types)]
    pub enum ClassicBpfOpcode {
        ALU_ADD_K = BPF_ALU | BPF_ADD | BPF_K,
        ALU_ADD_X = BPF_ALU | BPF_ADD | BPF_X,
        ALU_SUB_K = BPF_ALU | BPF_SUB | BPF_K,
        ALU_SUB_X = BPF_ALU | BPF_SUB | BPF_X,
        ALU_MUL_K = BPF_ALU | BPF_MUL | BPF_K,
        ALU_MUL_X = BPF_ALU | BPF_MUL | BPF_X,
        ALU_DIV_K = BPF_ALU | BPF_DIV | BPF_K,
        ALU_DIV_X = BPF_ALU | BPF_DIV | BPF_X,
        ALU_MOD_K = BPF_ALU | BPF_MOD | BPF_K,
        ALU_MOD_X = BPF_ALU | BPF_MOD | BPF_X,
        ALU_AND_K = BPF_ALU | BPF_AND | BPF_K,
        ALU_AND_X = BPF_ALU | BPF_AND | BPF_X,
        ALU_OR_K = BPF_ALU | BPF_OR | BPF_K,
        ALU_OR_X = BPF_ALU | BPF_OR | BPF_X,
        ALU_XOR_K = BPF_ALU | BPF_XOR | BPF_K,
        ALU_XOR_X = BPF_ALU | BPF_XOR | BPF_X,
        ALU_LSH_K = BPF_ALU | BPF_LSH | BPF_K,
        ALU_LSH_X = BPF_ALU | BPF_LSH | BPF_X,
        ALU_RSH_K = BPF_ALU | BPF_RSH | BPF_K,
        ALU_RSH_X = BPF_ALU | BPF_RSH | BPF_X,
        ALU_NEG = BPF_ALU | BPF_NEG,
        LD_W_ABS = BPF_LD | BPF_W | BPF_ABS,
        LD_H_ABS = BPF_LD | BPF_H | BPF_ABS,
        LD_B_ABS = BPF_LD | BPF_B | BPF_ABS,
        LD_W_LEN = BPF_LD | BPF_W | BPF_LEN,
        LD_W_IND = BPF_LD | BPF_W | BPF_IND,
        LD_H_IND = BPF_LD | BPF_H | BPF_IND,
        LD_B_IND = BPF_LD | BPF_B | BPF_IND,
        LD_IMM = BPF_LD | BPF_IMM,
        LD_MEM = BPF_LD | BPF_MEM,
        LDX_W_LEN = BPF_LDX | BPF_W | BPF_LEN,
        LDX_B_MSH = BPF_LDX | BPF_B | BPF_MSH,
        LDX_IMM = BPF_LDX | BPF_IMM,
        LDX_MEM = BPF_LDX | BPF_MEM,
        ST = BPF_ST,
        STX = BPF_STX,
        MISC_TAX = BPF_MISC | BPF_TAX,
        MISC_TXA = BPF_MISC | BPF_TXA,
        RET_K = BPF_RET | BPF_K,
        RET_A = BPF_RET | BPF_A,
        JMP_JA = BPF_JMP | BPF_JA,
        JMP_JEQ_K = BPF_JMP | BPF_JEQ | BPF_K,
        JMP_JEQ_X = BPF_JMP | BPF_JEQ | BPF_X,
        JMP_JGE_K = BPF_JMP | BPF_JGE | BPF_K,
        JMP_JGE_X = BPF_JMP | BPF_JGE | BPF_X,
        JMP_JGT_K = BPF_JMP | BPF_JGT | BPF_K,
        JMP_JGT_X = BPF_JMP | BPF_JGT | BPF_X,
        JMP_JSET_K = BPF_JMP | BPF_JSET | BPF_K,
        JMP_JSET_X = BPF_JMP | BPF_JSET | BPF_X,
        // LDX_W_ABS allowed only in seccomp!
        // should be rejected by the verifier on load from user regardless of context (netfilter/seccomp)
        // if the seperate opcode representation for seccomp is rejected, this will be an edge case
        // LDX_W_ABS = BPF_LDX | BPF_W | BPF_ABS,
    }

    // should seccomp opcodes have a distinct representation from regular netfilter cbpf?
    #[repr(u16)]
    #[derive(Debug, Clone)]
    #[expect(non_camel_case_types)]
    pub enum SeccompOpcode {
        cBPF(ClassicBpfOpcode),
        // there is one seccomp specific opcode, translated from LD_W_ABS in verify_seccomp()
        LDX_W_ABS = BPF_LDX | BPF_W | BPF_ABS,
    }
}
