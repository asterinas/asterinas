use crate::arch::vm::vcpu::VcpuArchInner;
use crate::prelude::*;

impl VcpuArchInner {
    pub(super) fn get_sbi_ext(&self, _reg_id: usize) -> usize {
        todo!("OneReg: get_sbi_ext is not impled!")
    }
    
    pub(super) fn set_sbi_ext(&self, _reg_id: usize, _reg_val: usize) {
        todo!("OneReg: set_sbi_ext is not impled!")
    }

    pub(super) fn copy_sbi_ext_reg_vec(&self, mut _vec: Option<&mut Vec<usize>>) -> usize {
        // todo!("OneReg: copy_sbi_ext_reg_vec is not impled!")
        0
    }

    pub(super) fn num_sbi_ext_regs(&self) -> usize {
        // todo!("OneReg: num_sbi_ext_regs is not impled")
        0
    }
}
