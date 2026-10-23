// SPDX-License-Identifier: MPL-2.0

//! Verified seccomp classic BPF programs and their interpreter.

mod interpreter;
mod verifier;

use crate::prelude::*;

pub(super) const MAX_INSNS: usize = 4096;
const MEM_WORDS: usize = 16;

/// A program whose instructions, control flow and scratch reads are validated.
pub(super) struct VerifiedProgram {
    instructions: Vec<Instruction>,
}

impl VerifiedProgram {
    pub(super) fn len(&self) -> usize {
        self.instructions.len()
    }
}

struct Instruction {
    op: Opcode,
    jt: u8,
    jf: u8,
    k: u32,
}

// Only the encodings accepted by Linux's seccomp_check_filter are represented.
// In particular, packet loads, ancillary loads and MOD are not seccomp opcodes.
#[repr(u16)]
#[derive(Clone, Copy, Debug, TryFromInt)]
enum Opcode {
    LdImm = 0x00,
    LdxImm = 0x01,
    St = 0x02,
    Stx = 0x03,
    AddK = 0x04,
    Ja = 0x05,
    RetK = 0x06,
    Tax = 0x07,
    AddX = 0x0c,
    SubK = 0x14,
    JeqK = 0x15,
    RetA = 0x16,
    SubX = 0x1c,
    JeqX = 0x1d,
    LdAbs = 0x20,
    MulK = 0x24,
    JgtK = 0x25,
    MulX = 0x2c,
    JgtX = 0x2d,
    DivK = 0x34,
    JgeK = 0x35,
    DivX = 0x3c,
    JgeX = 0x3d,
    OrK = 0x44,
    JsetK = 0x45,
    OrX = 0x4c,
    JsetX = 0x4d,
    AndK = 0x54,
    AndX = 0x5c,
    LdMem = 0x60,
    LdxMem = 0x61,
    LshK = 0x64,
    LshX = 0x6c,
    RshK = 0x74,
    RshX = 0x7c,
    LdLen = 0x80,
    LdxLen = 0x81,
    Neg = 0x84,
    Txa = 0x87,
    XorK = 0xa4,
    XorX = 0xac,
}
