use alloc::sync::Arc;
use kxos_frame::vm::{Vaddr, VmPerm, VmSpace};

use super::vm_page::VmPageRange;

pub const USER_STACK_BASE: Vaddr = 0x0000_0000_1000_0000;
pub const USER_STACK_SIZE: usize = 0x1000 * 16; // 64KB

pub struct UserStack {
    pub stack_bottom: Vaddr,
    stack_size: usize,
    vm_space: Option<Arc<VmSpace>>,
}

impl UserStack {
    // initialize user stack on fixed position
    pub const fn new(stack_bottom: Vaddr, stack_size: usize) -> Self {
        Self {
            stack_bottom,
            stack_size,
            vm_space: None,
        }
    }

    pub const fn new_default_config() -> Self {
        Self {
            stack_bottom: USER_STACK_BASE,
            stack_size: USER_STACK_SIZE,
            vm_space: None,
        }
    }

    pub fn map_and_zeroed(&self, vm_space: &VmSpace) {
        let mut vm_page_range =
            VmPageRange::new_range(self.stack_bottom..(self.stack_bottom + self.stack_size));
        let vm_perm = UserStack::perm();
        vm_page_range.map_zeroed(vm_space, vm_perm);
    }

    pub const fn perm() -> VmPerm {
        VmPerm::RWU
    }
}
