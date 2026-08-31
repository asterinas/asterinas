use alloc::vec::IntoIter;
use core::ops::{Deref, DerefMut};

use cbpf_opcodes::{
    AncOps, BPF_MAXINS, BPF_MEMWORDS, ClassicBpfOpcode, CommonCbpfOpcode, NetfilterBpfOpcode,
    SKF_AD_OFF, SeccompBpfOpcode,
};
use int_to_c_enum::TryFromIntError;
use ostd::{arch::cpu::context::UserContext, user::UserContextApi};

use crate::prelude::*;

/// Actions performed by the thread calling `seccomp` syscall
pub enum SeccompOp {
    SetModeStrict,
    SetModeFilter,
    // GetActionAvail,
    // GetNotifSizes
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum SeccompMode {
    Disabled = 0,
    Strict = 1,
    Filter = 2,
}

pub const SECCOMP_DATA_SIZE: u32 = 64;
pub const SECCOMP_RET_MASK: u32 = 0xffff0000;

#[repr(u32)]
#[derive(Debug, PartialEq, Eq, Clone, Copy, TryFromInt)]
pub enum SeccompRet {
    Kill = 0x00000000,
    Trap = 0x00030000,
    Errno = 0x00050000,
    Trace = 0x7ff00000,
    Allow = 0x7fff0000,
}

/// Seccomp policy state attached to a POSIX thread.
///
/// Created with [`SeccompMode::Disabled`] and no filter.
/// `leaf_filter` holds the most recently installed BPF filter program.
#[derive(Debug, Clone)]
pub struct SeccompState {
    pub mode: SeccompMode,
    pub leaf_filter: Option<Arc<SeccompFilterLeaf>>,
}

/// An iterator over the filter leaves of a [`SeccompState`].
#[derive(Clone, Debug)]
pub struct SeccompFilterIter<'a> {
    current: Option<&'a SeccompFilterLeaf>,
}

impl<'a> Iterator for SeccompFilterIter<'a> {
    type Item = &'a SeccompFilterLeaf;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current?;
        self.current = current.prev.as_deref();
        Some(current)
    }
}

impl<'a> IntoIterator for &'a SeccompState {
    type Item = &'a SeccompFilterLeaf;
    type IntoIter = SeccompFilterIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        SeccompFilterIter {
            current: self.leaf_filter.as_deref(),
        }
    }
}

/// cBPF instruction as passed from user space.
/// It has not yet been verified.
#[derive(Debug, Clone, Copy, Pod)]
pub struct RawFilterBlock {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

/// Unverified cBPF instruction vector.
#[derive(Debug, Clone)]
pub struct UnverifiedFilterProg(Vec<RawFilterBlock>);

impl Deref for UnverifiedFilterProg {
    type Target = Vec<RawFilterBlock>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for UnverifiedFilterProg {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoIterator for UnverifiedFilterProg {
    type Item = RawFilterBlock;
    type IntoIter = IntoIter<RawFilterBlock>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl UnverifiedFilterProg {
    pub fn new(len: usize) -> Self {
        Self(Vec::with_capacity(len))
    }
}

/// cBPF instruction verified for netfilter use.
#[derive(Debug, Clone)]
pub struct NetFilterBlock {
    pub code: ClassicBpfOpcode,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

impl TryFrom<RawFilterBlock> for NetFilterBlock {
    type Error = TryFromIntError;

    fn try_from(raw: RawFilterBlock) -> Result<Self, Self::Error> {
        let code = if let Ok(common) = CommonCbpfOpcode::try_from(raw.code) {
            ClassicBpfOpcode::common(common)
        } else if let Ok(net) = NetfilterBpfOpcode::try_from(raw.code) {
            ClassicBpfOpcode::netfilter(net)
        } else {
            return Err(TryFromIntError::InvalidValue);
        };
        Ok(Self {
            code,
            jt: raw.jt,
            jf: raw.jf,
            k: raw.k,
        })
    }
}

/// Verified cBPF instruction vector for netfilter use.
#[derive(Debug, Clone)]
pub struct NetFilterProg(Box<[NetFilterBlock]>);

impl Deref for NetFilterProg {
    type Target = [NetFilterBlock];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl NetFilterProg {
    // https://elixir.bootlin.com/linux/v6.18/source/net/core/filter.c#L1081
    pub fn from_unverified(unverified: UnverifiedFilterProg) -> Result<Self, Error> {
        use CommonCbpfOpcode::*;

        let len = unverified.len();

        // https://elixir.bootlin.com/linux/v6.18/source/net/core/filter.c#L1056
        if len == 0 || len > BPF_MAXINS {
            return Err(Error::new(Errno::EINVAL));
        }

        for (i, RawFilterBlock { code, jt, jf, k }) in unverified.iter().enumerate() {
            let valid: bool = if let Ok(common) = CommonCbpfOpcode::try_from(*code) {
                match common {
                    ALU_DIV_K | ALU_MOD_K => *k != 0,
                    ALU_LSH_K | ALU_RSH_K => *k < 32,
                    LD_MEM | LDX_MEM | ST | STX => *k < BPF_MEMWORDS,
                    JMP_JA => *k < (len - i - 1) as u32,
                    JMP_JEQ_K | JMP_JEQ_X | JMP_JGE_K | JMP_JGE_X | JMP_JGT_K | JMP_JGT_X
                    | JMP_JSET_K | JMP_JSET_X => {
                        i + (*jt as usize) + 1 < len && i + (*jf as usize) + 1 < len
                    }
                    _ => true,
                }
            } else if let Ok(net) = NetfilterBpfOpcode::try_from(*code) {
                match net {
                    NetfilterBpfOpcode::LD_W_ABS
                    | NetfilterBpfOpcode::LD_H_ABS
                    | NetfilterBpfOpcode::LD_B_ABS => {
                        *k < SKF_AD_OFF || AncOps::try_from(*k - SKF_AD_OFF).is_ok()
                    }
                    _ => true,
                }
            } else {
                false
            };

            if !valid {
                return Err(Error::new(Errno::EINVAL));
            }
        }

        let last_code = unverified[len - 1].code;
        if let Ok(common) = CommonCbpfOpcode::try_from(last_code) {
            match common {
                RET_K | RET_A => {
                    let blocks: Box<[NetFilterBlock]> = unverified
                        .into_iter()
                        .map(NetFilterBlock::try_from)
                        .collect::<Result<Vec<_>, _>>()?
                        .into_boxed_slice();
                    Self::check_load_and_stores(blocks)
                }
                _ => Err(Error::new(Errno::EINVAL)),
            }
        } else {
            Err(Error::new(Errno::EINVAL))
        }
    }

    // does it (and Linux) check for infinite loops? if not, does Linux allow infinite loops in cbpf?
    // if that's the case, should our verifier be more strict?
    // https://elixir.bootlin.com/linux/v6.18/source/kernel/seccomp.c#L278
    fn check_load_and_stores(unverified: Box<[NetFilterBlock]>) -> Result<Self> {
        use CommonCbpfOpcode::*;

        let mut memvalid: u16 = 0;
        let mut masks = vec![0xffffu16; unverified.len()];

        for (pc, ins) in unverified.iter().enumerate() {
            memvalid &= masks[pc];

            if let ClassicBpfOpcode::common(code) = ins.code {
                match code {
                    ST | STX => memvalid |= 1 << ins.k,
                    LD_MEM | LDX_MEM if (memvalid & (1 << ins.k)) == 0 => {
                        return Err(Error::new(Errno::EINVAL));
                    }
                    JMP_JA => {
                        masks[pc + 1 + (ins.k as usize)] &= memvalid;
                        memvalid = !0;
                    }
                    JMP_JEQ_K | JMP_JEQ_X | JMP_JGE_K | JMP_JGE_X | JMP_JGT_K | JMP_JGT_X
                    | JMP_JSET_K | JMP_JSET_X => {
                        masks[pc + 1 + (ins.jt as usize)] &= memvalid;
                        masks[pc + 1 + (ins.jf as usize)] &= memvalid;
                        memvalid = !0;
                    }
                    _ => (),
                }
            }
        }

        Ok(Self(unverified))
    }
}

/// cBPF instruction verified for seccomp use.
#[derive(Debug, Clone)]
pub struct SeccompFilterBlock {
    pub code: ClassicBpfOpcode,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

/// Verified cBPF instruction vector for seccomp use.
#[derive(Debug, Clone)]
pub struct SeccompFilterProg(Box<[SeccompFilterBlock]>);

impl Deref for SeccompFilterProg {
    type Target = [SeccompFilterBlock];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SeccompFilterProg {
    pub fn from_netfilter(netfilter: NetFilterProg) -> Result<Self, Error> {
        let insns = netfilter
            .iter()
            .map(Self::verify_and_map_seccomp)
            .collect::<Option<Vec<_>>>()
            .ok_or(Error::new(Errno::EINVAL))?;

        Ok(Self(insns.into_boxed_slice()))
    }

    // https://elixir.bootlin.com/linux/v6.18/source/kernel/seccomp.c#L278
    fn verify_and_map_seccomp(ins: &NetFilterBlock) -> Option<SeccompFilterBlock> {
        let code: ClassicBpfOpcode;
        let mut k = ins.k;

        match ins.code {
            ClassicBpfOpcode::common(common_code) => {
                code = ClassicBpfOpcode::common(common_code);
            }
            ClassicBpfOpcode::netfilter(net_code) => match net_code {
                NetfilterBpfOpcode::LD_W_ABS => {
                    if k >= SECCOMP_DATA_SIZE || k & 3 != 0 {
                        return None;
                    }
                    code = ClassicBpfOpcode::seccomp(SeccompBpfOpcode::LDX_W_ABS);
                }
                NetfilterBpfOpcode::LD_W_LEN => {
                    code = ClassicBpfOpcode::common(CommonCbpfOpcode::LD_IMM);
                    k = SECCOMP_DATA_SIZE;
                }
                NetfilterBpfOpcode::LDX_W_LEN => {
                    code = ClassicBpfOpcode::common(CommonCbpfOpcode::LDX_IMM);
                    k = SECCOMP_DATA_SIZE;
                }
                _ => return None,
            },
            ClassicBpfOpcode::seccomp(_) => return None,
        }

        Some(SeccompFilterBlock {
            code,
            jt: ins.jt,
            jf: ins.jf,
            k,
        })
    }
}

/// Seccomp filter program as a part of seccomp state tree.
/// May be owned by multiple threads.
#[derive(Debug, Clone)]
pub struct SeccompFilterLeaf {
    pub ins: SeccompFilterProg,
    pub prev: Option<Arc<SeccompFilterLeaf>>,
    // missing cache, log, notif related stuff, possibly others
}

impl SeccompState {
    /// Creates a new `SeccompState` with seccomp disabled and no filter loaded.
    pub fn new() -> Self {
        SeccompState {
            mode: SeccompMode::Disabled,
            leaf_filter: None,
        }
    }

    /// Returns an iterator over the filter leaves, from the most recently
    /// installed leaf to the oldest.
    pub fn iter(&self) -> SeccompFilterIter<'_> {
        self.into_iter()
    }
}

/// Abstracts over the fields of a single cBPF instruction so that
/// [`ClassicBPFilter::execute`] can work generically over both
/// [`NetFilterBlock`] and [`SeccompFilterBlock`].
pub trait ClassicBpfBlock {
    fn code(&self) -> ClassicBpfOpcode;
    fn jt(&self) -> u8;
    fn jf(&self) -> u8;
    fn k(&self) -> u32;
}

impl ClassicBpfBlock for NetFilterBlock {
    fn code(&self) -> ClassicBpfOpcode {
        self.code
    }
    fn jt(&self) -> u8 {
        self.jt
    }
    fn jf(&self) -> u8 {
        self.jf
    }
    fn k(&self) -> u32 {
        self.k
    }
}

impl ClassicBpfBlock for SeccompFilterBlock {
    fn code(&self) -> ClassicBpfOpcode {
        self.code
    }
    fn jt(&self) -> u8 {
        self.jt
    }
    fn jf(&self) -> u8 {
        self.jf
    }
    fn k(&self) -> u32 {
        self.k
    }
}

/// Trait for a cBPF program.
/// May need a rework (depending on what netfilter programs take as their arguments).
pub trait ClassicBPFilter: Deref<Target = [Self::Block]> {
    type Block: ClassicBpfBlock;

    fn execute(
        &self,
        user_ctx: &UserContext,
        syscall_id: u64,
        syscall_args: &[u64; 6],
    ) -> Result<u32>;
}

impl ClassicBPFilter for NetFilterProg {
    type Block = NetFilterBlock;

    /// TODO polymorphise args
    fn execute(
        &self,
        user_ctx: &UserContext,
        _syscall_id: u64,
        _syscall_args: &[u64; 6],
    ) -> Result<u32> {
        unimplemented!("Tried to execute netfilter under UserContext:'\n{user_ctx:#?}")
    }
}

impl ClassicBPFilter for SeccompFilterProg {
    type Block = SeccompFilterBlock;

    fn execute(
        &self,
        user_ctx: &UserContext,
        syscall_id: u64,
        syscall_args: &[u64; 6],
    ) -> Result<u32> {
        use cbpf_opcodes::CommonCbpfOpcode::*;

        let mut a: u32 = 0;
        let mut x: u32 = 0;
        let mut mem = [0u32; BPF_MEMWORDS as usize];
        let mut pc: usize = 0;

        while pc < self.len() {
            let ins = &self[pc];
            let ins_code = ins.code();
            let ins_jt = ins.jt();
            let ins_jf = ins.jf();
            let ins_k = ins.k();

            match ins_code {
                ClassicBpfOpcode::common(code) => match code {
                    RET_K => return Ok(ins_k),
                    RET_A => return Ok(a),
                    ALU_ADD_K => a = a.wrapping_add(ins_k),
                    ALU_ADD_X => a = a.wrapping_add(x),
                    ALU_SUB_K => a = a.wrapping_sub(ins_k),
                    ALU_SUB_X => a = a.wrapping_sub(x),
                    ALU_MUL_K => a = a.wrapping_mul(ins_k),
                    ALU_MUL_X => a = a.wrapping_mul(x),
                    ALU_DIV_K => {
                        if ins_k == 0 {
                            return Err(Error::new(Errno::EINVAL));
                        }
                        a = a.wrapping_div(ins_k);
                    }
                    ALU_DIV_X => {
                        if x == 0 {
                            return Err(Error::new(Errno::EINVAL));
                        }
                        a = a.wrapping_div(x);
                    }
                    ALU_MOD_K => {
                        if ins_k == 0 {
                            return Err(Error::new(Errno::EINVAL));
                        }
                        a = a.wrapping_rem(ins_k);
                    }
                    ALU_MOD_X => {
                        if x == 0 {
                            return Err(Error::new(Errno::EINVAL));
                        }
                        a = a.wrapping_rem(x);
                    }
                    ALU_AND_K => a &= ins_k,
                    ALU_AND_X => a &= x,
                    ALU_OR_K => a |= ins_k,
                    ALU_OR_X => a |= x,
                    ALU_XOR_K => a ^= ins_k,
                    ALU_XOR_X => a ^= x,
                    ALU_LSH_K => a = a.wrapping_shl(ins_k),
                    ALU_LSH_X => a = a.wrapping_shl(x),
                    ALU_RSH_K => a = a.wrapping_shr(ins_k),
                    ALU_RSH_X => a = a.wrapping_shr(x),
                    ALU_NEG => a = (!a).wrapping_add(1),
                    LD_IMM => a = ins_k,
                    LDX_IMM => x = ins_k,
                    LD_MEM => {
                        let index =
                            usize::try_from(ins_k).map_err(|_| Error::new(Errno::EINVAL))?;
                        a = mem[index];
                    }
                    LDX_MEM => {
                        let index =
                            usize::try_from(ins_k).map_err(|_| Error::new(Errno::EINVAL))?;
                        x = mem[index];
                    }
                    ST => {
                        let index =
                            usize::try_from(ins_k).map_err(|_| Error::new(Errno::EINVAL))?;
                        mem[index] = a;
                    }
                    STX => {
                        let index =
                            usize::try_from(ins_k).map_err(|_| Error::new(Errno::EINVAL))?;
                        mem[index] = x;
                    }
                    MISC_TAX => x = a,
                    MISC_TXA => a = x,
                    JMP_JA => {
                        pc = pc
                            .checked_add(1)
                            .and_then(|offset| offset.checked_add(ins_k as usize))
                            .ok_or_else(|| Error::new(Errno::EINVAL))?;
                        continue;
                    }
                    JMP_JEQ_K => {
                        pc = pc
                            .checked_add(1)
                            .and_then(|offset| {
                                Some(if a == ins_k {
                                    offset.checked_add(ins_jt as usize)?
                                } else {
                                    offset.checked_add(ins_jf as usize)?
                                })
                            })
                            .ok_or_else(|| Error::new(Errno::EINVAL))?;
                        continue;
                    }
                    JMP_JEQ_X => {
                        pc = pc
                            .checked_add(1)
                            .and_then(|offset| {
                                Some(if a == x {
                                    offset.checked_add(ins_jt as usize)?
                                } else {
                                    offset.checked_add(ins_jf as usize)?
                                })
                            })
                            .ok_or_else(|| Error::new(Errno::EINVAL))?;
                        continue;
                    }
                    JMP_JGE_K => {
                        pc = pc
                            .checked_add(1)
                            .and_then(|offset| {
                                Some(if a >= ins_k {
                                    offset.checked_add(ins_jt as usize)?
                                } else {
                                    offset.checked_add(ins_jf as usize)?
                                })
                            })
                            .ok_or_else(|| Error::new(Errno::EINVAL))?;
                        continue;
                    }
                    JMP_JGE_X => {
                        pc = pc
                            .checked_add(1)
                            .and_then(|offset| {
                                Some(if a >= x {
                                    offset.checked_add(ins_jt as usize)?
                                } else {
                                    offset.checked_add(ins_jf as usize)?
                                })
                            })
                            .ok_or_else(|| Error::new(Errno::EINVAL))?;
                        continue;
                    }
                    JMP_JGT_K => {
                        pc = pc
                            .checked_add(1)
                            .and_then(|offset| {
                                Some(if a > ins_k {
                                    offset.checked_add(ins_jt as usize)?
                                } else {
                                    offset.checked_add(ins_jf as usize)?
                                })
                            })
                            .ok_or_else(|| Error::new(Errno::EINVAL))?;
                        continue;
                    }
                    JMP_JGT_X => {
                        pc = pc
                            .checked_add(1)
                            .and_then(|offset| {
                                Some(if a > x {
                                    offset.checked_add(ins_jt as usize)?
                                } else {
                                    offset.checked_add(ins_jf as usize)?
                                })
                            })
                            .ok_or_else(|| Error::new(Errno::EINVAL))?;
                        continue;
                    }
                    JMP_JSET_K => {
                        pc = pc
                            .checked_add(1)
                            .and_then(|offset| {
                                Some(if (a & ins_k) != 0 {
                                    offset.checked_add(ins_jt as usize)?
                                } else {
                                    offset.checked_add(ins_jf as usize)?
                                })
                            })
                            .ok_or_else(|| Error::new(Errno::EINVAL))?;
                        continue;
                    }
                    JMP_JSET_X => {
                        pc = pc
                            .checked_add(1)
                            .and_then(|offset| {
                                Some(if (a & x) != 0 {
                                    offset.checked_add(ins_jt as usize)?
                                } else {
                                    offset.checked_add(ins_jf as usize)?
                                })
                            })
                            .ok_or_else(|| Error::new(Errno::EINVAL))?;
                        continue;
                    }
                },
                ClassicBpfOpcode::seccomp(SeccompBpfOpcode::LDX_W_ABS) => {
                    if ins_k >= SECCOMP_DATA_SIZE || ins_k & 3 != 0 {
                        return Err(Error::new(Errno::EINVAL));
                    }
                    x = {
                        match ins_k {
                            0 => Ok(syscall_id as u32),
                            4 => Ok(seccomp_arch()),
                            8 => Ok(user_ctx.instruction_pointer() as u32),
                            12 => Ok((user_ctx.instruction_pointer() >> 32) as u32),
                            16..=60 => {
                                let index = ((ins_k - 16) / 8) as usize;
                                let word_offset = ((ins_k - 16) % 8) / 4;
                                let arg = syscall_args[index];
                                Ok(((arg >> (word_offset * 32)) & 0xffffffff) as u32)
                            }
                            _ => Err(Error::new(Errno::EINVAL)),
                        }
                    }?;
                }
                ClassicBpfOpcode::netfilter(_) => return Err(Error::new(Errno::EINVAL)),
            }

            pc = pc.checked_add(1).ok_or_else(|| Error::new(Errno::EINVAL))?;
        }

        Err(Error::new(Errno::EINVAL))
    }
}

fn seccomp_arch() -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        0xc000003e
    }

    #[cfg(target_arch = "riscv64")]
    {
        0xf0000100
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "riscv64")))]
    {
        0
    }
}

pub mod cbpf_opcodes {
    use int_to_c_enum::TryFromInt;

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

    // Linux uses a -0x1000 literal with an unsigned type here.
    // It's an offset used for referencing ancillary data
    // 0xFFFF_F000u32 == -0x1000i32
    pub const SKF_AD_OFF: u32 = 0xFFFF_F000;

    #[repr(u32)]
    #[expect(non_camel_case_types)]
    #[derive(TryFromInt)]
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

    #[expect(unused)]
    /// TODO implement AncOps for NetFilter
    mod anc_params {
        pub const SKF_AD_MAX: u32 = 64;
        pub const SKF_NET_OFF: u32 = 1048576; // TODO: look at SKF_AD_OFF. -0x100000 in Linux
        pub const SKF_LL_OFF: u32 = 2097152; // -0x200000 in Linux
        pub const BPF_NET_OFF: u32 = SKF_NET_OFF;
        pub const BPF_LL_OFF: u32 = SKF_LL_OFF;
    }
    #[expect(clippy::upper_case_acronyms)]
    #[expect(non_camel_case_types)]
    #[repr(u16)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
    pub enum CommonCbpfOpcode {
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
        LD_IMM = BPF_LD | BPF_IMM,
        LD_MEM = BPF_LD | BPF_MEM,
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
    }

    #[expect(non_camel_case_types)]
    #[repr(u16)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
    pub enum NetfilterBpfOpcode {
        LD_W_ABS = BPF_LD | BPF_W | BPF_ABS,
        LD_H_ABS = BPF_LD | BPF_H | BPF_ABS,
        LD_B_ABS = BPF_LD | BPF_B | BPF_ABS,
        LD_W_LEN = BPF_LD | BPF_W | BPF_LEN,
        LD_W_IND = BPF_LD | BPF_W | BPF_IND,
        LD_H_IND = BPF_LD | BPF_H | BPF_IND,
        LD_B_IND = BPF_LD | BPF_B | BPF_IND,
        LDX_W_LEN = BPF_LDX | BPF_W | BPF_LEN,
        LDX_B_MSH = BPF_LDX | BPF_B | BPF_MSH,
    }

    #[expect(non_camel_case_types)]
    #[repr(u16)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
    pub enum SeccompBpfOpcode {
        LDX_W_ABS = BPF_LDX | BPF_W | BPF_ABS,
    }

    #[expect(non_camel_case_types)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ClassicBpfOpcode {
        common(CommonCbpfOpcode),
        netfilter(NetfilterBpfOpcode),
        seccomp(SeccompBpfOpcode),
    }
}
