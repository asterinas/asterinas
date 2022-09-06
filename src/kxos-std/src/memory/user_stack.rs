use kxos_frame::{
    config::PAGE_SIZE,
    vm::{Vaddr, VmPerm, VmSpace},
};

use super::vm_page::VmPageRange;

pub const USER_STACK_BASE: Vaddr = 0x0000_0000_1000_0000;
pub const USER_STACK_SIZE: usize = 0x1000 * 16; // 64KB

pub struct UserStack {
    /// The high address of user stack
    stack_top: Vaddr,
    stack_size: usize,
}

impl UserStack {
    /// initialize user stack on base addr
    pub const fn new(stack_top: Vaddr, stack_size: usize) -> Self {
        Self {
            stack_top,
            stack_size,
        }
    }

    /// This function only work for first process
    pub const fn new_default_config() -> Self {
        Self {
            // add a guard page at stack top
            stack_top: USER_STACK_BASE - PAGE_SIZE,
            stack_size: USER_STACK_SIZE,
        }
    }

    /// the user stack top(high address), used to setup rsp
    pub const fn stack_top(&self) -> Vaddr {
        self.stack_top
    }

    /// the user stack bottom(low address)
    const fn stack_bottom(&self) -> Vaddr {
        self.stack_top - self.stack_size
    }

    pub fn map_and_zeroed(&self, vm_space: &VmSpace) {
        let mut vm_page_range = VmPageRange::new_range(self.stack_bottom()..self.stack_top());
        let vm_perm = UserStack::perm();
        vm_page_range.map_zeroed(vm_space, vm_perm);
    }

    pub const fn perm() -> VmPerm {
        VmPerm::RWU
    }
}
