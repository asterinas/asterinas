// SPDX-License-Identifier: MPL-2.0

use super::{MEM_WORDS, Opcode, VerifiedProgram};
use crate::security::seccomp::abi::SeccompData;

impl VerifiedProgram {
    pub(in crate::security::seccomp) fn run(&self, data: &SeccompData) -> u32 {
        let mut accumulator = 0u32;
        let mut index = 0u32;
        let mut memory = [0u32; MEM_WORDS];
        let mut pc = 0;

        loop {
            let insn = &self.instructions[pc];
            pc += 1;
            let operand = match insn.op {
                Opcode::AddX
                | Opcode::SubX
                | Opcode::MulX
                | Opcode::DivX
                | Opcode::OrX
                | Opcode::AndX
                | Opcode::LshX
                | Opcode::RshX
                | Opcode::XorX
                | Opcode::JeqX
                | Opcode::JgtX
                | Opcode::JgeX
                | Opcode::JsetX => index,
                _ => insn.k,
            };

            match insn.op {
                Opcode::LdImm => accumulator = insn.k,
                Opcode::LdxImm => index = insn.k,
                Opcode::LdAbs => accumulator = data.word(insn.k),
                Opcode::LdLen => accumulator = size_of::<SeccompData>() as u32,
                Opcode::LdxLen => index = size_of::<SeccompData>() as u32,
                Opcode::LdMem => accumulator = memory[insn.k as usize],
                Opcode::LdxMem => index = memory[insn.k as usize],
                Opcode::St => memory[insn.k as usize] = accumulator,
                Opcode::Stx => memory[insn.k as usize] = index,
                Opcode::Tax => index = accumulator,
                Opcode::Txa => accumulator = index,
                Opcode::AddK | Opcode::AddX => accumulator = accumulator.wrapping_add(operand),
                Opcode::SubK | Opcode::SubX => accumulator = accumulator.wrapping_sub(operand),
                Opcode::MulK | Opcode::MulX => accumulator = accumulator.wrapping_mul(operand),
                Opcode::DivK | Opcode::DivX => {
                    // A runtime zero divisor returns zero from the entire BPF program.
                    if operand == 0 {
                        return 0;
                    }
                    accumulator /= operand;
                }
                Opcode::OrK | Opcode::OrX => accumulator |= operand,
                Opcode::AndK | Opcode::AndX => accumulator &= operand,
                Opcode::XorK | Opcode::XorX => accumulator ^= operand,
                // Linux converts these to ALU32 shifts, masking register counts to 5 bits.
                Opcode::LshK | Opcode::LshX => accumulator = accumulator.wrapping_shl(operand),
                Opcode::RshK | Opcode::RshX => accumulator = accumulator.wrapping_shr(operand),
                Opcode::Neg => accumulator = accumulator.wrapping_neg(),
                Opcode::Ja => pc += insn.k as usize,
                Opcode::JeqK
                | Opcode::JeqX
                | Opcode::JgtK
                | Opcode::JgtX
                | Opcode::JgeK
                | Opcode::JgeX
                | Opcode::JsetK
                | Opcode::JsetX => {
                    let condition = match insn.op {
                        Opcode::JeqK | Opcode::JeqX => accumulator == operand,
                        Opcode::JgtK | Opcode::JgtX => accumulator > operand,
                        Opcode::JgeK | Opcode::JgeX => accumulator >= operand,
                        Opcode::JsetK | Opcode::JsetX => accumulator & operand != 0,
                        _ => unreachable!(),
                    };
                    pc += if condition { insn.jt } else { insn.jf } as usize;
                }
                Opcode::RetK => return insn.k,
                Opcode::RetA => return accumulator,
            }
        }
    }
}
