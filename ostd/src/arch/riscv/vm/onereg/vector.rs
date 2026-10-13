use crate::arch::vm::vcpu::VcpuArchInner;
use crate::prelude::*;

impl VcpuArchInner {
    pub(super) fn get_vector(&self, _reg_id: usize) -> usize {
        todo!("OneReg: get_vector is not impled!")
    }
    
    pub(super) fn set_vector(&self, _reg_id: usize, _reg_val: usize) {
        todo!("OneReg: set_vector is not impled!")
    }

    pub(super) fn copy_vector_reg_vec(&self, mut _vec: Option<&mut Vec<usize>>) -> usize {
        // todo!("OneReg: copy_vector_reg_vec is not impled!")
        0
    }

    pub(super) fn num_vector_regs(&self) -> usize {
        // todo!("OneReg: num_vector_regs is not impled")
        0
    }
}
