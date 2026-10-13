use crate::arch::vm::vcpu::VcpuArchInner;
use crate::prelude::*;
use crate::arch::vm::sbi::state_reg_count;

impl VcpuArchInner {
    pub(super) fn get_sbi_state(&self, _reg_id: usize) -> usize {
        todo!("OneReg: get_sbi_state is not impled!")
    }
    
    pub(super) fn set_sbi_state(&self, _reg_id: usize, _reg_val: usize) {
        todo!("OneReg: set_sbi_state is not impled!")
    }

    pub(super) fn copy_sbi_state_reg_vec(&self, vec: Option<&mut Vec<usize>>) -> usize {
        state_reg_count(self, vec)
    }

    pub(super) fn num_sbi_state_regs(&self) -> usize {
        self.copy_sbi_state_reg_vec(None)
    }
}
