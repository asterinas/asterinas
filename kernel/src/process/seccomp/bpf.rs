// SPDX-License-Identifier: MPL-2.0

//! The classic-BPF (cBPF) subset that seccomp filters are written in.
//!
//! Seccomp filters are read-only cBPF programs: they observe a [`SeccompData`]
//! descriptor of the system call and return a 32-bit action, with no access to
//! packets, maps, or memory writes beyond the 16-word scratch area. [`run_filter`]
//! interprets that subset and [`validate_filter`] rejects malformed programs at
//! install time, so an accepted program can never reach an undefined state while
//! a system call is on the hot path.
//!
//! The opcodes and encodings follow Linux `linux/filter.h`,
//! `linux/bpf_common.h`, and `Documentation/networking/filter.rst`.

use super::{SECCOMP_RET_KILL_PROCESS, SeccompData, SockFilter};
use crate::prelude::*;

/// The maximum number of instructions in a seccomp filter (`BPF_MAXINSNS`).
pub const BPF_MAXINSNS: usize = 4096;

/// Validates a classic-BPF program before it is installed as a seccomp filter.
///
/// A program is accepted only if it is non-empty and within [`BPF_MAXINSNS`],
/// uses only the supported read-only opcodes, keeps every jump target, scratch
/// slot, and `seccomp_data` load offset in range, and ends in a `BPF_RET`.
/// Rejecting these statically lets [`run_filter`] stay branch-light and means a
/// validated program cannot fault at evaluation time.
pub fn validate_filter(program: &[SockFilter]) -> Result<()> {
    if program.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "empty BPF program");
    }
    if program.len() > BPF_MAXINSNS {
        return_errno_with_message!(Errno::EINVAL, "BPF program is too large");
    }

    for (idx, insn) in program.iter().enumerate() {
        validate_instruction(insn)?;

        let next_idx = idx + 1;
        if is_cond_jump(insn.code) {
            validate_jump_target(next_idx, insn.jt as usize, program.len())?;
            validate_jump_target(next_idx, insn.jf as usize, program.len())?;
        } else if insn.code == bpf_jmp_k(BPF_JA) {
            validate_jump_target(next_idx, insn.k as usize, program.len())?;
        }
    }

    let Some(last) = program.last() else {
        return_errno_with_message!(Errno::EINVAL, "empty BPF program");
    };
    if (last.code & BPF_CLASS_MASK) != BPF_RET {
        return_errno_with_message!(Errno::EINVAL, "BPF program does not end with RET");
    }

    check_load_and_stores(program)?;

    Ok(())
}

/// Rejects programs that read a scratch-memory slot before it is written on some
/// path. This mirrors Linux's `check_load_and_stores`: a per-instruction mask of
/// initialized slots is propagated across the (forward-only) jumps, and a
/// `BPF_LD|BPF_MEM`/`BPF_LDX|BPF_MEM` from a slot that is not provably
/// initialized is `EINVAL`. Run after instruction/jump validation, so memory
/// slots and jump targets are already in range.
fn check_load_and_stores(program: &[SockFilter]) -> Result<()> {
    // Each bit of a mask marks a scratch slot as initialized; bits start set and
    // are cleared as paths that leave a slot uninitialized merge in.
    let mut masks = vec![u16::MAX; program.len()];
    let mut valid = 0u16;

    for (pc, insn) in program.iter().enumerate() {
        valid &= masks[pc];

        let code = insn.code;
        if code == bpf_st() || code == bpf_stx() {
            valid |= 1 << insn.k;
        } else if code == bpf_ld_mem(BPF_W) || code == bpf_ldx_mem(BPF_W) {
            if valid & (1 << insn.k) == 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "BPF load from uninitialized scratch memory"
                );
            }
        } else if code == bpf_jmp_k(BPF_JA) {
            masks[pc + 1 + insn.k as usize] &= valid;
            valid = u16::MAX;
        } else if is_cond_jump(code) {
            masks[pc + 1 + insn.jt as usize] &= valid;
            masks[pc + 1 + insn.jf as usize] &= valid;
            valid = u16::MAX;
        }
    }

    Ok(())
}

/// Evaluates a validated classic-BPF program against `data` and returns the raw
/// 32-bit seccomp action it produces.
///
/// The program counter, accumulator, index register, and 16-word scratch area
/// follow the classic-BPF machine. Any state that a validated program cannot
/// reach (for example an unsupported opcode) fails closed by returning
/// `SECCOMP_RET_KILL_PROCESS`.
pub fn run_filter(program: &[SockFilter], data: &SeccompData) -> u32 {
    let mut acc = 0u32;
    let mut x = 0u32;
    let mut mem = [0u32; BPF_MEMWORDS];
    let mut pc = 0usize;

    loop {
        let Some(insn) = program.get(pc) else {
            return SECCOMP_RET_KILL_PROCESS;
        };
        pc += 1;

        match insn.code {
            code if code == bpf_ld_abs(BPF_W) => {
                let Some(val) = load_seccomp_word(data, insn.k) else {
                    return SECCOMP_RET_KILL_PROCESS;
                };
                acc = val;
            }
            code if code == bpf_ld_abs(BPF_H) || code == bpf_ld_abs(BPF_B) => {
                return SECCOMP_RET_KILL_PROCESS;
            }
            code if code == bpf_ld_imm(BPF_W) => {
                acc = insn.k;
            }
            code if code == bpf_ld_len(BPF_W) => {
                return SECCOMP_RET_KILL_PROCESS;
            }
            code if code == bpf_ld_mem(BPF_W) => {
                let Some(val) = mem.get(insn.k as usize) else {
                    return SECCOMP_RET_KILL_PROCESS;
                };
                acc = *val;
            }
            code if code == bpf_ldx_imm(BPF_W) => {
                x = insn.k;
            }
            code if code == bpf_ldx_mem(BPF_W) => {
                let Some(val) = mem.get(insn.k as usize) else {
                    return SECCOMP_RET_KILL_PROCESS;
                };
                x = *val;
            }
            code if code == bpf_st() => {
                let Some(slot) = mem.get_mut(insn.k as usize) else {
                    return SECCOMP_RET_KILL_PROCESS;
                };
                *slot = acc;
            }
            code if code == bpf_stx() => {
                let Some(slot) = mem.get_mut(insn.k as usize) else {
                    return SECCOMP_RET_KILL_PROCESS;
                };
                *slot = x;
            }
            code if code == bpf_alu_k(BPF_ADD) => acc = acc.wrapping_add(insn.k),
            code if code == bpf_alu_x(BPF_ADD) => acc = acc.wrapping_add(x),
            code if code == bpf_alu_k(BPF_SUB) => acc = acc.wrapping_sub(insn.k),
            code if code == bpf_alu_x(BPF_SUB) => acc = acc.wrapping_sub(x),
            code if code == bpf_alu_k(BPF_MUL) => acc = acc.wrapping_mul(insn.k),
            code if code == bpf_alu_x(BPF_MUL) => acc = acc.wrapping_mul(x),
            code if code == bpf_alu_k(BPF_DIV) => {
                if insn.k == 0 {
                    return SECCOMP_RET_KILL_PROCESS;
                }
                acc /= insn.k;
            }
            code if code == bpf_alu_x(BPF_DIV) => {
                if x == 0 {
                    return SECCOMP_RET_KILL_PROCESS;
                }
                acc /= x;
            }
            code if code == bpf_alu_k(BPF_OR) => acc |= insn.k,
            code if code == bpf_alu_x(BPF_OR) => acc |= x,
            code if code == bpf_alu_k(BPF_AND) => acc &= insn.k,
            code if code == bpf_alu_x(BPF_AND) => acc &= x,
            code if code == bpf_alu_k(BPF_LSH) => acc = acc.wrapping_shl(insn.k),
            code if code == bpf_alu_x(BPF_LSH) => acc = acc.wrapping_shl(x),
            code if code == bpf_alu_k(BPF_RSH) => acc = acc.wrapping_shr(insn.k),
            code if code == bpf_alu_x(BPF_RSH) => acc = acc.wrapping_shr(x),
            code if code == bpf_alu_k(BPF_MOD) => {
                if insn.k == 0 {
                    return SECCOMP_RET_KILL_PROCESS;
                }
                acc %= insn.k;
            }
            code if code == bpf_alu_x(BPF_MOD) => {
                if x == 0 {
                    return SECCOMP_RET_KILL_PROCESS;
                }
                acc %= x;
            }
            code if code == bpf_alu_k(BPF_XOR) => acc ^= insn.k,
            code if code == bpf_alu_x(BPF_XOR) => acc ^= x,
            code if code == bpf_alu_neg() => acc = 0u32.wrapping_sub(acc),
            code if code == bpf_jmp_k(BPF_JA) => pc = pc.saturating_add(insn.k as usize),
            code if code == bpf_jmp_k(BPF_JEQ) => {
                pc += if acc == insn.k {
                    insn.jt as usize
                } else {
                    insn.jf as usize
                };
            }
            code if code == bpf_jmp_x(BPF_JEQ) => {
                pc += if acc == x {
                    insn.jt as usize
                } else {
                    insn.jf as usize
                };
            }
            code if code == bpf_jmp_k(BPF_JGT) => {
                pc += if acc > insn.k {
                    insn.jt as usize
                } else {
                    insn.jf as usize
                };
            }
            code if code == bpf_jmp_x(BPF_JGT) => {
                pc += if acc > x {
                    insn.jt as usize
                } else {
                    insn.jf as usize
                };
            }
            code if code == bpf_jmp_k(BPF_JGE) => {
                pc += if acc >= insn.k {
                    insn.jt as usize
                } else {
                    insn.jf as usize
                };
            }
            code if code == bpf_jmp_x(BPF_JGE) => {
                pc += if acc >= x {
                    insn.jt as usize
                } else {
                    insn.jf as usize
                };
            }
            code if code == bpf_jmp_k(BPF_JSET) => {
                pc += if acc & insn.k != 0 {
                    insn.jt as usize
                } else {
                    insn.jf as usize
                };
            }
            code if code == bpf_jmp_x(BPF_JSET) => {
                pc += if acc & x != 0 {
                    insn.jt as usize
                } else {
                    insn.jf as usize
                };
            }
            code if code == bpf_ret_k() => return insn.k,
            code if code == bpf_ret_a() => return acc,
            code if code == bpf_misc_tax() => x = acc,
            code if code == bpf_misc_txa() => acc = x,
            _ => return SECCOMP_RET_KILL_PROCESS,
        }
    }
}

fn validate_instruction(insn: &SockFilter) -> Result<()> {
    match insn.code {
        code if code == bpf_ld_abs(BPF_W) => validate_seccomp_load_offset(insn.k),
        code if code == bpf_ld_imm(BPF_W) => Ok(()),
        code if code == bpf_ld_mem(BPF_W)
            || code == bpf_ldx_mem(BPF_W)
            || code == bpf_st()
            || code == bpf_stx() =>
        {
            validate_mem_slot(insn.k)
        }
        code if code == bpf_ldx_imm(BPF_W) => Ok(()),
        code if is_alu(code) => validate_alu_instruction(insn),
        code if is_jump(code) => {
            if code == bpf_jmp_k(BPF_JA) {
                return Ok(());
            }
            if !is_cond_jump(code) {
                return_errno_with_message!(Errno::EINVAL, "unsupported BPF jump instruction");
            }
            Ok(())
        }
        code if code == bpf_ret_k() || code == bpf_ret_a() => Ok(()),
        code if code == bpf_misc_tax() || code == bpf_misc_txa() => Ok(()),
        _ => Err(Error::with_message(
            Errno::EINVAL,
            "unsupported seccomp BPF instruction",
        )),
    }
}

fn validate_alu_instruction(insn: &SockFilter) -> Result<()> {
    match insn.code {
        code if code == bpf_alu_k(BPF_DIV) && insn.k == 0 => {
            return_errno_with_message!(Errno::EINVAL, "BPF division by zero")
        }
        code if code == bpf_alu_k(BPF_MOD) && insn.k == 0 => {
            return_errno_with_message!(Errno::EINVAL, "BPF modulo by zero")
        }
        code if (code == bpf_alu_k(BPF_LSH) || code == bpf_alu_k(BPF_RSH)) && insn.k >= 32 => {
            return_errno_with_message!(Errno::EINVAL, "BPF shift is out of range")
        }
        code if code == bpf_alu_k(BPF_ADD)
            || code == bpf_alu_x(BPF_ADD)
            || code == bpf_alu_k(BPF_SUB)
            || code == bpf_alu_x(BPF_SUB)
            || code == bpf_alu_k(BPF_MUL)
            || code == bpf_alu_x(BPF_MUL)
            || code == bpf_alu_x(BPF_DIV)
            || code == bpf_alu_k(BPF_OR)
            || code == bpf_alu_x(BPF_OR)
            || code == bpf_alu_k(BPF_AND)
            || code == bpf_alu_x(BPF_AND)
            || code == bpf_alu_x(BPF_LSH)
            || code == bpf_alu_x(BPF_RSH)
            || code == bpf_alu_x(BPF_MOD)
            || code == bpf_alu_k(BPF_XOR)
            || code == bpf_alu_x(BPF_XOR)
            || code == bpf_alu_neg() =>
        {
            Ok(())
        }
        _ => return_errno_with_message!(Errno::EINVAL, "unsupported BPF ALU instruction"),
    }
}

fn validate_jump_target(next_idx: usize, offset: usize, program_len: usize) -> Result<()> {
    let Some(target) = next_idx.checked_add(offset) else {
        return_errno_with_message!(Errno::EINVAL, "BPF jump target overflows");
    };
    if target >= program_len {
        return_errno_with_message!(Errno::EINVAL, "BPF jump target is out of bounds");
    }
    Ok(())
}

fn validate_seccomp_load_offset(offset: u32) -> Result<()> {
    if load_seccomp_word(&SeccompData::default(), offset).is_none() {
        return_errno_with_message!(Errno::EINVAL, "invalid seccomp_data load offset");
    }
    Ok(())
}

fn validate_mem_slot(slot: u32) -> Result<()> {
    if slot as usize >= BPF_MEMWORDS {
        return_errno_with_message!(Errno::EINVAL, "invalid BPF scratch memory slot");
    }
    Ok(())
}

fn load_seccomp_word(data: &SeccompData, offset: u32) -> Option<u32> {
    match offset {
        0 => Some(data.nr as u32),
        4 => Some(data.arch),
        8 => Some(data.instruction_pointer as u32),
        12 => Some((data.instruction_pointer >> 32) as u32),
        16 => Some(data.args[0] as u32),
        20 => Some((data.args[0] >> 32) as u32),
        24 => Some(data.args[1] as u32),
        28 => Some((data.args[1] >> 32) as u32),
        32 => Some(data.args[2] as u32),
        36 => Some((data.args[2] >> 32) as u32),
        40 => Some(data.args[3] as u32),
        44 => Some((data.args[3] >> 32) as u32),
        48 => Some(data.args[4] as u32),
        52 => Some((data.args[4] >> 32) as u32),
        56 => Some(data.args[5] as u32),
        60 => Some((data.args[5] >> 32) as u32),
        _ => None,
    }
}

fn is_alu(code: u16) -> bool {
    (code & BPF_CLASS_MASK) == BPF_ALU
}

fn is_jump(code: u16) -> bool {
    (code & BPF_CLASS_MASK) == BPF_JMP
}

fn is_cond_jump(code: u16) -> bool {
    matches!(
        code,
        c if c == bpf_jmp_k(BPF_JEQ)
            || c == bpf_jmp_x(BPF_JEQ)
            || c == bpf_jmp_k(BPF_JGT)
            || c == bpf_jmp_x(BPF_JGT)
            || c == bpf_jmp_k(BPF_JGE)
            || c == bpf_jmp_x(BPF_JGE)
            || c == bpf_jmp_k(BPF_JSET)
            || c == bpf_jmp_x(BPF_JSET)
    )
}

const BPF_CLASS_MASK: u16 = 0x07;
const BPF_LD: u16 = 0x00;
const BPF_LDX: u16 = 0x01;
const BPF_ST: u16 = 0x02;
const BPF_STX: u16 = 0x03;
const BPF_ALU: u16 = 0x04;
const BPF_JMP: u16 = 0x05;
const BPF_RET: u16 = 0x06;
const BPF_MISC: u16 = 0x07;

const BPF_W: u16 = 0x00;
const BPF_H: u16 = 0x08;
const BPF_B: u16 = 0x10;

const BPF_IMM: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_MEM: u16 = 0x60;
const BPF_LEN: u16 = 0x80;

const BPF_ADD: u16 = 0x00;
const BPF_SUB: u16 = 0x10;
const BPF_MUL: u16 = 0x20;
const BPF_DIV: u16 = 0x30;
const BPF_OR: u16 = 0x40;
const BPF_AND: u16 = 0x50;
const BPF_LSH: u16 = 0x60;
const BPF_RSH: u16 = 0x70;
const BPF_NEG: u16 = 0x80;
const BPF_MOD: u16 = 0x90;
const BPF_XOR: u16 = 0xa0;

const BPF_JA: u16 = 0x00;
const BPF_JEQ: u16 = 0x10;
const BPF_JGT: u16 = 0x20;
const BPF_JGE: u16 = 0x30;
const BPF_JSET: u16 = 0x40;

const BPF_K: u16 = 0x00;
const BPF_X: u16 = 0x08;
const BPF_A: u16 = 0x10;

const BPF_TAX: u16 = 0x00;
const BPF_TXA: u16 = 0x80;
const BPF_MEMWORDS: usize = 16;

const fn bpf_ld_abs(size: u16) -> u16 {
    BPF_LD | size | BPF_ABS
}

const fn bpf_ld_imm(size: u16) -> u16 {
    BPF_LD | size | BPF_IMM
}

const fn bpf_ld_len(size: u16) -> u16 {
    BPF_LD | size | BPF_LEN
}

const fn bpf_ld_mem(size: u16) -> u16 {
    BPF_LD | size | BPF_MEM
}

const fn bpf_ldx_imm(size: u16) -> u16 {
    BPF_LDX | size | BPF_IMM
}

const fn bpf_ldx_mem(size: u16) -> u16 {
    BPF_LDX | size | BPF_MEM
}

const fn bpf_st() -> u16 {
    BPF_ST
}

const fn bpf_stx() -> u16 {
    BPF_STX
}

const fn bpf_alu_k(op: u16) -> u16 {
    BPF_ALU | op | BPF_K
}

const fn bpf_alu_x(op: u16) -> u16 {
    BPF_ALU | op | BPF_X
}

const fn bpf_alu_neg() -> u16 {
    BPF_ALU | BPF_NEG
}

const fn bpf_jmp_k(op: u16) -> u16 {
    BPF_JMP | op | BPF_K
}

const fn bpf_jmp_x(op: u16) -> u16 {
    BPF_JMP | op | BPF_X
}

const fn bpf_ret_k() -> u16 {
    BPF_RET | BPF_K
}

const fn bpf_ret_a() -> u16 {
    BPF_RET | BPF_A
}

const fn bpf_misc_tax() -> u16 {
    BPF_MISC | BPF_TAX
}

const fn bpf_misc_txa() -> u16 {
    BPF_MISC | BPF_TXA
}

/// Builds a single-instruction `BPF_RET | BPF_K` program returning `value`,
/// used by the seccomp filter-chain and state tests in the parent module.
#[cfg(ktest)]
pub(super) fn ret_program(value: u32) -> Box<[SockFilter]> {
    Box::new([SockFilter {
        code: bpf_ret_k(),
        jt: 0,
        jf: 0,
        k: value,
    }])
}

/// Builds a valid `len`-instruction program: `len - 1` harmless loads of the
/// syscall number followed by `BPF_RET value`. Used by the chain-length test.
#[cfg(ktest)]
pub(super) fn padded_program(len: usize, value: u32) -> Box<[SockFilter]> {
    let mut program = Vec::with_capacity(len);
    for _ in 1..len {
        program.push(SockFilter {
            code: bpf_ld_abs(BPF_W),
            jt: 0,
            jf: 0,
            k: 0,
        });
    }
    program.push(SockFilter {
        code: bpf_ret_k(),
        jt: 0,
        jf: 0,
        k: value,
    });
    program.into_boxed_slice()
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::{
        super::{AUDIT_ARCH_NATIVE, SECCOMP_RET_ALLOW, SECCOMP_RET_ERRNO},
        *,
    };

    fn stmt(code: u16, k: u32) -> SockFilter {
        SockFilter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }

    fn jump(code: u16, k: u32, jt: u8, jf: u8) -> SockFilter {
        SockFilter { code, jt, jf, k }
    }

    fn sample_data(nr: i32) -> SeccompData {
        SeccompData {
            nr,
            arch: AUDIT_ARCH_NATIVE,
            instruction_pointer: 0x1234_5678_9abc_def0,
            args: [1, 2, 3, 4, 5, 6],
        }
    }

    #[ktest]
    fn seccomp() {
        seccomp_allow_program_runs();
        seccomp_syscall_whitelist_can_return_errno();
        seccomp_arch_check_can_kill_mismatches();
        seccomp_alu_memory_and_ret_a_work();
        seccomp_alu_and_index_register_opcodes_evaluate();
        seccomp_conditional_jumps_select_the_taken_branch();
        seccomp_x_form_conditional_jumps_evaluate();
        seccomp_data_word_loads_can_filter_arguments();
        seccomp_invalid_programs_are_rejected();
        seccomp_ja_zero_can_fall_through_to_next_instruction();
    }

    #[ktest]
    fn seccomp_allow_program_runs() {
        let program = [stmt(bpf_ret_k(), SECCOMP_RET_ALLOW)];
        assert!(validate_filter(&program).is_ok());
        assert_eq!(run_filter(&program, &sample_data(39)), SECCOMP_RET_ALLOW);
    }

    #[ktest]
    fn seccomp_alu_and_index_register_opcodes_evaluate() {
        // A = (nr + 10) & 0xff, exercising an ALU K op, TAX, an immediate load,
        // an ALU X op, and RET A.
        let program = [
            stmt(bpf_ld_abs(BPF_W), 0),
            stmt(bpf_alu_k(BPF_ADD), 10),
            stmt(bpf_misc_tax(), 0),
            stmt(bpf_ld_imm(BPF_W), 0xff),
            stmt(bpf_alu_x(BPF_AND), 0),
            stmt(bpf_ret_a(), 0),
        ];
        assert!(validate_filter(&program).is_ok());
        assert_eq!(run_filter(&program, &sample_data(5)), 15);

        // X-form comparison: TXA loads X into A, then `JGT X` compares the
        // loaded syscall number against X.
        let program = [
            stmt(bpf_ldx_imm(BPF_W), 100),
            stmt(bpf_misc_txa(), 0),
            stmt(bpf_ld_abs(BPF_W), 0),
            jump(bpf_jmp_x(BPF_JGT), 0, 0, 1),
            stmt(bpf_ret_k(), SECCOMP_RET_ALLOW),
            stmt(bpf_ret_k(), SECCOMP_RET_ERRNO),
        ];
        assert!(validate_filter(&program).is_ok());
        assert_eq!(run_filter(&program, &sample_data(200)), SECCOMP_RET_ALLOW);
        assert_eq!(run_filter(&program, &sample_data(50)), SECCOMP_RET_ERRNO);
    }

    #[ktest]
    fn seccomp_conditional_jumps_select_the_taken_branch() {
        // Each program loads the syscall number, runs one conditional jump that
        // falls through to ALLOW when taken and skips to ERRNO otherwise.
        let eval = |code: u16, k: u32, nr: i32| {
            let program = [
                stmt(bpf_ld_abs(BPF_W), 0),
                jump(code, k, 0, 1),
                stmt(bpf_ret_k(), SECCOMP_RET_ALLOW),
                stmt(bpf_ret_k(), SECCOMP_RET_ERRNO),
            ];
            assert!(validate_filter(&program).is_ok());
            run_filter(&program, &sample_data(nr))
        };

        assert_eq!(eval(bpf_jmp_k(BPF_JEQ), 5, 5), SECCOMP_RET_ALLOW);
        assert_eq!(eval(bpf_jmp_k(BPF_JEQ), 6, 5), SECCOMP_RET_ERRNO);
        assert_eq!(eval(bpf_jmp_k(BPF_JGT), 3, 5), SECCOMP_RET_ALLOW);
        assert_eq!(eval(bpf_jmp_k(BPF_JGT), 5, 5), SECCOMP_RET_ERRNO);
        assert_eq!(eval(bpf_jmp_k(BPF_JGE), 5, 5), SECCOMP_RET_ALLOW);
        assert_eq!(eval(bpf_jmp_k(BPF_JGE), 6, 5), SECCOMP_RET_ERRNO);
        assert_eq!(eval(bpf_jmp_k(BPF_JSET), 0x4, 5), SECCOMP_RET_ALLOW);
        assert_eq!(eval(bpf_jmp_k(BPF_JSET), 0x8, 5), SECCOMP_RET_ERRNO);
    }

    #[ktest]
    fn seccomp_x_form_conditional_jumps_evaluate() {
        // Load args[0] into X and args[1] into A, then branch on A vs X.
        let eval = |code: u16, a: u64, x: u64| {
            let program = [
                stmt(bpf_ld_abs(BPF_W), 16),
                stmt(bpf_misc_tax(), 0),
                stmt(bpf_ld_abs(BPF_W), 24),
                jump(code, 0, 0, 1),
                stmt(bpf_ret_k(), SECCOMP_RET_ALLOW),
                stmt(bpf_ret_k(), SECCOMP_RET_ERRNO),
            ];
            assert!(validate_filter(&program).is_ok());
            let mut data = sample_data(0);
            data.args = [x, a, 0, 0, 0, 0];
            run_filter(&program, &data)
        };

        assert_eq!(eval(bpf_jmp_x(BPF_JEQ), 5, 5), SECCOMP_RET_ALLOW);
        assert_eq!(eval(bpf_jmp_x(BPF_JEQ), 5, 6), SECCOMP_RET_ERRNO);
        assert_eq!(eval(bpf_jmp_x(BPF_JGE), 7, 5), SECCOMP_RET_ALLOW);
        assert_eq!(eval(bpf_jmp_x(BPF_JGE), 4, 5), SECCOMP_RET_ERRNO);
        assert_eq!(eval(bpf_jmp_x(BPF_JSET), 0x6, 0x4), SECCOMP_RET_ALLOW);
        assert_eq!(eval(bpf_jmp_x(BPF_JSET), 0x1, 0x4), SECCOMP_RET_ERRNO);
    }

    #[ktest]
    fn seccomp_data_word_loads_can_filter_arguments() {
        // Deny based on the first syscall argument, the way real profiles match
        // arguments: ERRNO when args[0] == 42, ALLOW otherwise.
        let program = [
            stmt(bpf_ld_abs(BPF_W), 16),
            jump(bpf_jmp_k(BPF_JEQ), 42, 0, 1),
            stmt(bpf_ret_k(), SECCOMP_RET_ERRNO | Errno::EPERM as u32),
            stmt(bpf_ret_k(), SECCOMP_RET_ALLOW),
        ];
        assert!(validate_filter(&program).is_ok());
        let mut data = sample_data(0);
        data.args = [42, 0, 0, 0, 0, 0];
        assert_eq!(
            run_filter(&program, &data),
            SECCOMP_RET_ERRNO | Errno::EPERM as u32
        );
        data.args[0] = 7;
        assert_eq!(run_filter(&program, &data), SECCOMP_RET_ALLOW);

        // The high 32 bits of a 64-bit field are a separate word load.
        let high_word = [stmt(bpf_ld_abs(BPF_W), 20), stmt(bpf_ret_a(), 0)];
        assert!(validate_filter(&high_word).is_ok());
        let mut data = sample_data(0);
        data.args[0] = 0x1234_5678_9abc_def0;
        assert_eq!(run_filter(&high_word, &data), 0x1234_5678);
    }

    #[ktest]
    fn seccomp_syscall_whitelist_can_return_errno() {
        let program = [
            stmt(bpf_ld_abs(BPF_W), 0),
            jump(bpf_jmp_k(BPF_JEQ), 1, 0, 1),
            stmt(bpf_ret_k(), SECCOMP_RET_ALLOW),
            stmt(bpf_ret_k(), SECCOMP_RET_ERRNO | Errno::EPERM as u32),
        ];
        assert!(validate_filter(&program).is_ok());
        assert_eq!(run_filter(&program, &sample_data(1)), SECCOMP_RET_ALLOW);
        assert_eq!(
            run_filter(&program, &sample_data(39)),
            SECCOMP_RET_ERRNO | Errno::EPERM as u32
        );
    }

    #[ktest]
    fn seccomp_arch_check_can_kill_mismatches() {
        let program = [
            stmt(bpf_ld_abs(BPF_W), 4),
            jump(bpf_jmp_k(BPF_JEQ), AUDIT_ARCH_NATIVE, 1, 0),
            stmt(bpf_ret_k(), SECCOMP_RET_KILL_PROCESS),
            stmt(bpf_ret_k(), SECCOMP_RET_ALLOW),
        ];
        assert!(validate_filter(&program).is_ok());
        assert_eq!(run_filter(&program, &sample_data(0)), SECCOMP_RET_ALLOW);

        let mut data = sample_data(0);
        data.arch = 0;
        assert_eq!(run_filter(&program, &data), SECCOMP_RET_KILL_PROCESS);
    }

    #[ktest]
    fn seccomp_alu_memory_and_ret_a_work() {
        let program = [
            stmt(bpf_ld_imm(BPF_W), SECCOMP_RET_ERRNO),
            stmt(bpf_st(), 0),
            stmt(bpf_ld_imm(BPF_W), Errno::EACCES as u32),
            stmt(bpf_ldx_mem(BPF_W), 0),
            stmt(bpf_alu_x(BPF_OR), 0),
            stmt(bpf_ret_a(), 0),
        ];
        assert!(validate_filter(&program).is_ok());
        assert_eq!(
            run_filter(&program, &sample_data(0)),
            SECCOMP_RET_ERRNO | Errno::EACCES as u32
        );
    }

    #[ktest]
    fn seccomp_invalid_programs_are_rejected() {
        assert!(validate_filter(&[]).is_err());
        assert!(validate_filter(&[stmt(bpf_ld_abs(BPF_W), 64), stmt(bpf_ret_a(), 0)]).is_err());
        assert!(
            validate_filter(&[jump(bpf_jmp_k(BPF_JA), 4, 0, 0), stmt(bpf_ret_k(), 0)]).is_err()
        );
        assert!(validate_filter(&[stmt(bpf_ld_imm(BPF_W), 0)]).is_err());
        assert!(validate_filter(&[stmt(bpf_ld_abs(BPF_H), 0), stmt(bpf_ret_a(), 0)]).is_err());

        // Reading a scratch slot before it is written is rejected, but reading
        // it after a store is accepted.
        assert!(validate_filter(&[stmt(bpf_ld_mem(BPF_W), 0), stmt(bpf_ret_a(), 0)]).is_err());
        assert!(
            validate_filter(&[
                stmt(bpf_ld_imm(BPF_W), 1),
                stmt(bpf_st(), 0),
                stmt(bpf_ld_mem(BPF_W), 0),
                stmt(bpf_ret_a(), 0)
            ])
            .is_ok()
        );
    }

    #[ktest]
    fn seccomp_ja_zero_can_fall_through_to_next_instruction() {
        let program = [
            jump(bpf_jmp_k(BPF_JA), 0, 0, 0),
            stmt(bpf_ret_k(), SECCOMP_RET_ALLOW),
        ];
        assert!(validate_filter(&program).is_ok());
        assert_eq!(run_filter(&program, &sample_data(0)), SECCOMP_RET_ALLOW);
    }
}
