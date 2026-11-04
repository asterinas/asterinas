// SPDX-License-Identifier: MPL-2.0

use super::vcpu::VcpuArch;
use crate::kvm::run_page::KvmRun;
use crate::kvm::ioctl::KVM_RUN_MMIO_DATA_OFFSET;
use crate::prelude::*;

const INSN_MATCH_LB: usize = 0x3;
const INSN_MASK_LB: usize = 0x707f;
const INSN_MATCH_LH: usize = 0x1003;
const INSN_MASK_LH: usize = 0x707f;
const INSN_MATCH_LW: usize = 0x2003;
const INSN_MASK_LW: usize = 0x707f;
const INSN_MATCH_LD: usize = 0x3003;
const INSN_MASK_LD: usize = 0x707f;
const INSN_MATCH_LBU: usize = 0x4003;
const INSN_MASK_LBU: usize = 0x707f;
const INSN_MATCH_LHU: usize = 0x5003;
const INSN_MASK_LHU: usize = 0x707f;
const INSN_MATCH_LWU: usize = 0x6003;
const INSN_MASK_LWU: usize = 0x707f;
const INSN_MATCH_SB: usize = 0x23;
const INSN_MASK_SB: usize = 0x707f;
const INSN_MATCH_SH: usize = 0x1023;
const INSN_MASK_SH: usize = 0x707f;
const INSN_MATCH_SW: usize = 0x2023;
const INSN_MASK_SW: usize = 0x707f;
const INSN_MATCH_SD: usize = 0x3023;
const INSN_MASK_SD: usize = 0x707f;
const INSN_MATCH_C_LD: usize = 0x6000;
const INSN_MASK_C_LD: usize = 0xe003;
const INSN_MATCH_C_SD: usize = 0xe000;
const INSN_MASK_C_SD: usize = 0xe003;
const INSN_MATCH_C_LW: usize = 0x4000;
const INSN_MASK_C_LW: usize = 0xe003;
const INSN_MATCH_C_SW: usize = 0xc000;
const INSN_MASK_C_SW: usize = 0xe003;
const INSN_MATCH_C_LDSP: usize = 0x6002;
const INSN_MASK_C_LDSP: usize = 0xe003;
const INSN_MATCH_C_SDSP: usize = 0xe002;
const INSN_MASK_C_SDSP: usize = 0xe003;
const INSN_MATCH_C_LWSP: usize = 0x4002;
const INSN_MASK_C_LWSP: usize = 0xe003;
const INSN_MATCH_C_SWSP: usize = 0xc002;
const INSN_MASK_C_SWSP: usize = 0xe003;

const SH_RD: usize = 7;
const SH_RS2C: usize = 2;

const INSN_16BIT_MASK: usize = 0x3;
const LOG_REGBYTES: usize = 3;
const REG_MASK: usize = (1 << (5 + LOG_REGBYTES)) - (1 << LOG_REGBYTES);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Load,
    Store,
}

/// Whether need sign-extend（only for load）
#[derive(Clone, Copy, PartialEq, Eq)]
enum Extend {
    Sign,
    Zero,
}

struct InsnDecode {
    matcher: usize,
    mask: usize,
    len: i32,
    access: Access,
    compressed: bool,
    extend: Extend,
}

/// Table-driven instruction decoding table. To add a new command, just append a line here.
///
/// Description:
/// - `Sign` means sign extension by `len`; `Zero` means zero extension.
/// - For store instructions, the `extend` field is meaningless (fill in `Zero`).
const INSN_TABLE: &[InsnDecode] = &[
    // ---- Standard load ----
    InsnDecode { matcher: INSN_MATCH_LB,   mask: INSN_MASK_LB,   len: 1, access: Access::Load,  compressed: false, extend: Extend::Sign },
    InsnDecode { matcher: INSN_MATCH_LH,   mask: INSN_MASK_LH,   len: 2, access: Access::Load,  compressed: false, extend: Extend::Sign },
    InsnDecode { matcher: INSN_MATCH_LW,   mask: INSN_MASK_LW,   len: 4, access: Access::Load,  compressed: false, extend: Extend::Sign },
    InsnDecode { matcher: INSN_MATCH_LD,   mask: INSN_MASK_LD,   len: 8, access: Access::Load,  compressed: false, extend: Extend::Sign },
    InsnDecode { matcher: INSN_MATCH_LBU,  mask: INSN_MASK_LBU,  len: 1, access: Access::Load,  compressed: false, extend: Extend::Zero },
    InsnDecode { matcher: INSN_MATCH_LHU,  mask: INSN_MASK_LHU,  len: 2, access: Access::Load,  compressed: false, extend: Extend::Zero },
    InsnDecode { matcher: INSN_MATCH_LWU,  mask: INSN_MASK_LWU,  len: 4, access: Access::Load,  compressed: false, extend: Extend::Zero },
    // ---- Standard store ----
    InsnDecode { matcher: INSN_MATCH_SB,   mask: INSN_MASK_SB,   len: 1, access: Access::Store, compressed: false, extend: Extend::Zero },
    InsnDecode { matcher: INSN_MATCH_SH,   mask: INSN_MASK_SH,   len: 2, access: Access::Store, compressed: false, extend: Extend::Zero },
    InsnDecode { matcher: INSN_MATCH_SW,   mask: INSN_MASK_SW,   len: 4, access: Access::Store, compressed: false, extend: Extend::Zero },
    InsnDecode { matcher: INSN_MATCH_SD,   mask: INSN_MASK_SD,   len: 8, access: Access::Store, compressed: false, extend: Extend::Zero },
    // ---- RVC load ----
    InsnDecode { matcher: INSN_MATCH_C_LD,   mask: INSN_MASK_C_LD,   len: 8, access: Access::Load,  compressed: true,  extend: Extend::Sign },
    InsnDecode { matcher: INSN_MATCH_C_LW,   mask: INSN_MASK_C_LW,   len: 4, access: Access::Load,  compressed: true,  extend: Extend::Sign },
    InsnDecode { matcher: INSN_MATCH_C_LDSP, mask: INSN_MASK_C_LDSP, len: 8, access: Access::Load,  compressed: true,  extend: Extend::Sign },
    InsnDecode { matcher: INSN_MATCH_C_LWSP, mask: INSN_MASK_C_LWSP, len: 4, access: Access::Load,  compressed: true,  extend: Extend::Sign },
    // ---- RVC store ----
    InsnDecode { matcher: INSN_MATCH_C_SD,   mask: INSN_MASK_C_SD,   len: 8, access: Access::Store, compressed: true,  extend: Extend::Zero },
    InsnDecode { matcher: INSN_MATCH_C_SW,   mask: INSN_MASK_C_SW,   len: 4, access: Access::Store, compressed: true,  extend: Extend::Zero },
    InsnDecode { matcher: INSN_MATCH_C_SDSP, mask: INSN_MASK_C_SDSP, len: 8, access: Access::Store, compressed: true,  extend: Extend::Zero },
    InsnDecode { matcher: INSN_MATCH_C_SWSP, mask: INSN_MASK_C_SWSP, len: 4, access: Access::Store, compressed: true,  extend: Extend::Zero },
];

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct MmioDecode {
    pub(super) insn: usize,
    pub(super) insn_len: i32,
    pub(super) len: i32,
    pub(super) shift: i32,
    pub(super) return_handled: i32,
}

impl MmioDecode {
    pub(super) fn reset_with_insn(&mut self, trap_insn: usize) {
        self.insn = 0;
        self.insn_len = 0;
        self.len = 0;
        self.shift = 0;

        // Determine trapped instruction
        if trap_insn & 0x1 != 0 {
            // Bit[0] == 1 implies trapped instruction value is
            // transformed instruction or custom instruction.
            self.insn = trap_insn | INSN_16BIT_MASK;
            if trap_insn & 0x1 << 1 != 0 {
                self.insn_len = super::insn::insn_len(self.insn) as i32;
            } else {
                self.insn_len = 2;
            }
        }
    }

    fn lookup(&self) -> Option<&'static InsnDecode> {
        INSN_TABLE
            .iter()
            .find(|d| self.insn & d.mask == d.matcher)
    }

    fn shift_for(len: i32) -> i32 {
        8 * (size_of::<usize>() as i32 - len)
    }

    /// Unified decoding of load instructions: set `len` / `shift`, 
    /// and register re-encoding in RVC form
    fn decode_load(&mut self) -> Option<&'static InsnDecode> {
        let desc = self.lookup()?;
        if desc.access != Access::Load {
            return None;
        }

        // RVC SP relative form requires rd != 0, otherwise it is illegal encoding
        if matches!(desc.matcher, INSN_MATCH_C_LDSP | INSN_MATCH_C_LWSP)
            && (self.insn >> SH_RD) & 0x1f == 0
        {
            return None;
        }

        self.len = desc.len;
        // Only sign-extend instructions require shift processing; 
        // shift remains 0 when zero-extended
        self.shift = if desc.extend == Extend::Sign {
            Self::shift_for(desc.len)
        } else {
            0
        };

        // RVC non-SP form: Encode rs2' in the compressed form to the rd bit 
        // to facilitate subsequent unified access to registers
        if desc.compressed && !matches!(desc.matcher, INSN_MATCH_C_LDSP | INSN_MATCH_C_LWSP) {
            self.insn = self.rvc_rs2s() << SH_RD;
        }

        Some(desc)
    }

    /// Unified decoding of store instructions: set `len` and take out the data to be written
    fn decode_store(&mut self, arch: &VcpuArch) -> Result<(usize, &'static InsnDecode)> {
        let desc = self
            .lookup()
            .ok_or_else(|| Error::new(Errno::EOPNOTSUPP))?;
        if desc.access != Access::Store {
            return Err(Error::new(Errno::EOPNOTSUPP));
        }

        // RVC SP relative form requires rs2 != 0, otherwise it is illegal encoding
        if matches!(desc.matcher, INSN_MATCH_C_SDSP | INSN_MATCH_C_SWSP)
            && (self.insn >> SH_RD) & 0x1f == 0
        {
            return Err(Error::new(Errno::EOPNOTSUPP));
        }

        self.len = desc.len;

        let data = if desc.compressed {
            self.get_rs2s(arch)?
        } else {
            self.get_rs2(arch)?
        };

        Ok((data, desc))
    }

    /// Decode length of MMIO and shift
    pub(super) fn load_decode(&mut self) {
        let _ = self.decode_load();
        self.return_handled = 0;
    }

    /// Decode length of MMIO and data
    pub(super) fn store_decode(&mut self, arch: &VcpuArch) -> Result<usize> {
        let (data, _desc) = self.decode_store(arch)?;
        self.return_handled = 0;
        Ok(data)
    }

    /// Handle the data from user
    pub(super) fn handle_return(&self, run: Arc<KvmRun>, arch: &VcpuArch) -> Result<()> {
        let index = (self.insn >> SH_RD) & 0x1F;

        // Read a usize from the MMIO data area uniformly, and then press 
        // len/shift for truncation and sign extension
        let data = run.read_run_val::<usize>(KVM_RUN_MMIO_DATA_OFFSET)?;

        let wdata = match self.len {
            1 | 2 | 4 | 8 => {
                let masked = if self.len == 8 {
                    data
                } else {
                    data & ((1usize << (8 * self.len)) - 1)
                };
                (((masked << self.shift) as isize) >> self.shift) as usize
            }
            _ => return Err(Error::new(Errno::EOPNOTSUPP)),
        };

        arch.set_general_reg(index, wdata)?;
        Ok(())
    }

    /// Get the rs2 reg_num of RVC insn
    fn rvc_rs2s(&self) -> usize {
        8 + self.rv_x(SH_RS2C, 3)
    }

    fn rv_x(&self, s: usize, n: usize) -> usize {
        self.rv_x_mask(s, (1 << (n)) - 1)
    }

    fn rv_x_mask(&self, s: usize, mask: usize) -> usize {
        ((self.insn) >> (s)) & (mask)
    }

    /// Get the rs2 reg_val of insn
    fn get_rs2(&self, arch: &VcpuArch) -> Result<usize> {
        Ok(arch.general_reg((((self.insn) >> 17) & REG_MASK) / size_of::<usize>())?)
    }

    /// Get the rs2 reg_val of RVC insn
    fn get_rs2s(&self, arch: &VcpuArch) -> Result<usize> {
        Ok(arch.general_reg((((self.rvc_rs2s() << 7) << 3) & REG_MASK) / size_of::<usize>())?)
    }
}
