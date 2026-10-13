use ostd::arch::vm::insn_len;
use ostd::arch::vm::VcpuContext;

pub const INSN_MATCH_LB: usize = 0x3;
pub const INSN_MASK_LB: usize = 0x707f;
pub const INSN_MATCH_LH: usize = 0x1003;
pub const INSN_MASK_LH: usize = 0x707f;
pub const INSN_MATCH_LW: usize = 0x2003;
pub const INSN_MASK_LW: usize = 0x707f;
pub const INSN_MATCH_LD: usize = 0x3003;
pub const INSN_MASK_LD: usize = 0x707f;
pub const INSN_MATCH_LBU: usize = 0x4003;
pub const INSN_MASK_LBU: usize = 0x707f;
pub const INSN_MATCH_LHU: usize = 0x5003;
pub const INSN_MASK_LHU: usize = 0x707f;
pub const INSN_MATCH_LWU: usize = 0x6003;
pub const INSN_MASK_LWU: usize = 0x707f;
pub const INSN_MATCH_SB: usize = 0x23;
pub const INSN_MASK_SB: usize = 0x707f;
pub const INSN_MATCH_SH: usize = 0x1023;
pub const INSN_MASK_SH: usize = 0x707f;
pub const INSN_MATCH_SW: usize = 0x2023;
pub const INSN_MASK_SW: usize = 0x707f;
pub const INSN_MATCH_SD: usize = 0x3023;
pub const INSN_MASK_SD: usize = 0x707f;

pub const INSN_MATCH_C_LD: usize = 0x6000;
pub const INSN_MASK_C_LD: usize = 0xe003;
pub const INSN_MATCH_C_SD: usize = 0xe000;
pub const INSN_MASK_C_SD: usize = 0xe003;
pub const INSN_MATCH_C_LW: usize = 0x4000;
pub const INSN_MASK_C_LW: usize = 0xe003;
pub const INSN_MATCH_C_SW: usize = 0xc000;
pub const INSN_MASK_C_SW: usize = 0xe003;
pub const INSN_MATCH_C_LDSP: usize = 0x6002;
pub const INSN_MASK_C_LDSP: usize = 0xe003;
pub const INSN_MATCH_C_SDSP: usize = 0xe002;
pub const INSN_MASK_C_SDSP: usize = 0xe003;
pub const INSN_MATCH_C_LWSP: usize = 0x4002;
pub const INSN_MASK_C_LWSP: usize = 0xe003;
pub const INSN_MATCH_C_SWSP: usize = 0xc002;
pub const INSN_MASK_C_SWSP: usize = 0xe003;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct MmioDecode {
	pub insn: usize,
	pub insn_len: i32,
	pub len: i32,
	pub shift: i32,
	pub return_handled: i32,
}

impl MmioDecode {
    pub fn reset_with_htinst(&mut self, htinst: usize) {
        self.insn = 0;
        self.insn_len = 0;
        self.len = 0;
        self.shift = 0;

        if htinst & 0x1 != 0 {
		    self.insn = htinst | 0x3;
            if htinst & 0x1 << 1 != 0 {
                self.insn_len = insn_len(self.insn) as i32;
            } else {
                self.insn_len = 2;
            }
	    }
    }

    pub fn load_decode(&mut self) {
        if self.insn & INSN_MASK_LW == INSN_MATCH_LW {
            self.len = 4;
            self.shift = 8 * (size_of::<usize>() as i32 - self.len);
        } else if self.insn & INSN_MASK_LB == INSN_MATCH_LB {
            self.len = 1;
            self.shift = 8 * (size_of::<usize>() as i32 - self.len);
        } else if self.insn & INSN_MASK_LBU == INSN_MATCH_LBU {
            self.len = 1;
        } else if self.insn & INSN_MASK_LD == INSN_MATCH_LD {
            self.len = 8;
            self.shift = 8 * (size_of::<usize>() as i32 - self.len);
        } else if self.insn & INSN_MASK_LWU == INSN_MATCH_LWU {
            self.len = 4;
        } else if self.insn & INSN_MASK_LH == INSN_MATCH_LH {
            self.len = 2;
            self.shift = 8 * (size_of::<usize>() as i32 - self.len);
        } else if self.insn & INSN_MASK_LHU == INSN_MATCH_LHU {
            self.len = 2;
        } else if self.insn & INSN_MASK_C_LD == INSN_MATCH_C_LD {
            self.len = 8;
            self.shift = 8 * (size_of::<usize>() as i32 - self.len);
            self.insn = (8 + (((self.insn) >> (2)) & (7))) << 7;
        } else if self.insn & INSN_MASK_C_LDSP == INSN_MATCH_C_LDSP && (self.insn >> 7) & 0x1f != 0 {
            self.len = 8;
            self.shift = 8 * (size_of::<usize>() as i32 - self.len);
        } else if self.insn & INSN_MASK_C_LW == INSN_MATCH_C_LW {
            self.len = 4;
            self.shift = 8 * (size_of::<usize>() as i32 - self.len);
            self.insn = (8 + (((self.insn) >> (2)) & (7))) << 7;
        } else if self.insn & INSN_MASK_C_LWSP == INSN_MATCH_C_LWSP && (self.insn >> 7) & 0x1f != 0 {
            self.len = 4;
            self.shift = 8 * (size_of::<usize>() as i32 - self.len);
        }

        self.return_handled = 0;
    }

    pub fn store_decode(&mut self, ctx: &VcpuContext) -> usize {
        // Here /8 is / sizeof(usize)
        let mut data = ctx.get_reg(((((self.insn) >> 17) & 0xF8) as u64) / 8).unwrap();

        if self.insn & INSN_MASK_SW == INSN_MATCH_SW {
            self.len = 4;
        } else if self.insn & INSN_MASK_SB == INSN_MATCH_SB {
            self.len = 1;
        } else if self.insn & INSN_MASK_SD == INSN_MATCH_SD {
            self.len = 8;
        } else if self.insn & INSN_MASK_SH == INSN_MATCH_SH {
            self.len = 2;
        } else if self.insn & INSN_MASK_C_SD == INSN_MATCH_C_SD {
            self.len = 8;
            data = ctx.get_reg((((((8 + (((self.insn) >> (2)) & (7))) << 7) << 3) & 0xF8) as u64) / 8 ).unwrap();
        } else if self.insn & INSN_MASK_C_SDSP == INSN_MATCH_C_SDSP && (self.insn >> 7) & 0x1f != 0 {
            self.len = 8;
            data = ctx.get_reg((((((8 + (((self.insn) >> (2)) & (7))) << 7) << 3) & 0xF8) as u64) / 8 ).unwrap();
        } else if self.insn & INSN_MASK_C_SW == INSN_MATCH_C_SW {
            self.len = 4;
            data = ctx.get_reg((((((8 + (((self.insn) >> (2)) & (7))) << 7) << 3) & 0xF8) as u64) / 8 ).unwrap();
        } else if self.insn & INSN_MASK_C_SWSP == INSN_MATCH_C_SWSP && (self.insn >> 7) & 0x1f != 0 {
            self.len = 4;
            data = ctx.get_reg((((((8 + (((self.insn) >> (2)) & (7))) << 7) << 3) & 0xF8) as u64) / 8 ).unwrap();
        }
        self.return_handled = 0;
        data
    }
}
