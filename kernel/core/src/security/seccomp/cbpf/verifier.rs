// SPDX-License-Identifier: MPL-2.0

use super::{Instruction, MAX_INSNS, MEM_WORDS, Opcode, VerifiedProgram};
use crate::{
    prelude::*,
    security::seccomp::abi::{SeccompData, SockFilter},
};

// Acceptance rules follow seccomp_check_filter() and bpf_check_classic():
// <https://github.com/torvalds/linux/blob/v6.18/kernel/seccomp.c>
// <https://github.com/torvalds/linux/blob/v6.18/net/core/filter.c>.

impl VerifiedProgram {
    pub(in crate::security::seccomp) fn new(raw: Vec<SockFilter>) -> Result<Self> {
        if raw.is_empty() || raw.len() > MAX_INSNS {
            return_errno_with_message!(Errno::EINVAL, "invalid seccomp program length");
        }

        let mut instructions = Vec::new();
        instructions.try_reserve_exact(raw.len()).map_err(|_| {
            Error::with_message(Errno::ENOMEM, "cannot allocate seccomp instructions")
        })?;

        for (pc, insn) in raw.iter().enumerate() {
            let op = Opcode::try_from(insn.code)
                .map_err(|_| Error::with_message(Errno::EINVAL, "invalid seccomp opcode"))?;
            check_instruction(op, insn, raw.len() - pc - 1)?;
            instructions.push(Instruction {
                op,
                jt: insn.jt,
                jf: insn.jf,
                k: insn.k,
            });
        }

        if !matches!(instructions.last().unwrap().op, Opcode::RetK | Opcode::RetA) {
            return_errno_with_message!(Errno::EINVAL, "seccomp program must end with a return");
        }
        check_scratch_memory(&instructions)?;
        Ok(Self { instructions })
    }
}

fn check_instruction(op: Opcode, insn: &SockFilter, remaining: usize) -> Result<()> {
    let is_valid = match op {
        Opcode::LdAbs => insn.k < size_of::<SeccompData>() as u32 && insn.k.is_multiple_of(4),
        Opcode::DivK => insn.k != 0,
        Opcode::LshK | Opcode::RshK => insn.k < u32::BITS,
        Opcode::LdMem | Opcode::LdxMem | Opcode::St | Opcode::Stx => insn.k < MEM_WORDS as u32,
        Opcode::Ja => (insn.k as usize) < remaining,
        Opcode::JeqK
        | Opcode::JeqX
        | Opcode::JgtK
        | Opcode::JgtX
        | Opcode::JgeK
        | Opcode::JgeX
        | Opcode::JsetK
        | Opcode::JsetX => (insn.jt as usize) < remaining && (insn.jf as usize) < remaining,
        _ => true,
    };
    if !is_valid {
        return_errno_with_message!(Errno::EINVAL, "invalid seccomp instruction operand");
    }
    Ok(())
}

fn check_scratch_memory(instructions: &[Instruction]) -> Result<()> {
    let mut masks = Vec::new();
    masks.try_reserve_exact(instructions.len()).map_err(|_| {
        Error::with_message(Errno::ENOMEM, "cannot allocate seccomp verifier state")
    })?;
    masks.resize(instructions.len(), u16::MAX);
    let mut initialized = 0u16;

    // Match Linux's forward intersection analysis, including unreachable code.
    // Every path arriving at a load must have initialized the scratch word.
    for (pc, insn) in instructions.iter().enumerate() {
        initialized &= masks[pc];
        match insn.op {
            Opcode::St | Opcode::Stx => initialized |= 1 << insn.k,
            Opcode::LdMem | Opcode::LdxMem => {
                if initialized & (1 << insn.k) == 0 {
                    return_errno_with_message!(Errno::EINVAL, "uninitialized seccomp scratch word");
                }
            }
            Opcode::Ja => {
                masks[pc + 1 + insn.k as usize] &= initialized;
                initialized = u16::MAX;
            }
            Opcode::JeqK
            | Opcode::JeqX
            | Opcode::JgtK
            | Opcode::JgtX
            | Opcode::JgeK
            | Opcode::JgeX
            | Opcode::JsetK
            | Opcode::JsetX => {
                masks[pc + 1 + insn.jt as usize] &= initialized;
                masks[pc + 1 + insn.jf as usize] &= initialized;
                initialized = u16::MAX;
            }
            _ => {}
        }
    }
    Ok(())
}
