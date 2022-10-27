//! This module defines the process initial stack.
//! The process initial stack, contains arguments, environmental variables and auxiliary vectors
//! The data layout of init stack can be seen in Figure 3.9 in https://uclibc.org/docs/psABI-x86_64.pdf

use crate::{memory::vm_page::VmPageRange, prelude::*};
use core::mem;
use kxos_frame::{
    vm::{VmIo, VmPerm, VmSpace},
    AlignExt,
};

use super::elf::ElfHeaderInfo;
use super::{
    aux_vec::{AuxKey, AuxVec},
    elf::ElfError,
};

pub const INIT_STACK_BASE: Vaddr = 0x0000_0000_2000_0000;
pub const INIT_STACK_SIZE: usize = 0x1000 * 16; // 64KB

/*
 * The initial stack of a process looks like below(This figure is from occlum):
 *
 *
 *  +---------------------+ <------+ Top of stack
 *  |                     |          (high address)
 *  | Null-terminated     |
 *  | strings referenced  |
 *  | by variables below  |
 *  |                     |
 *  +---------------------+
 *  | AT_NULL             |
 *  +---------------------+
 *  | AT_NULL             |
 *  +---------------------+
 *  | ...                 |
 *  +---------------------+
 *  | aux_val[0]          |
 *  +---------------------+
 *  | aux_key[0]          | <------+ Auxiliary table
 *  +---------------------+
 *  | NULL                |
 *  +---------------------+
 *  | ...                 |
 *  +---------------------+
 *  | char* envp[0]       | <------+ Environment variables
 *  +---------------------+
 *  | NULL                |
 *  +---------------------+
 *  | char* argv[argc-1]  |
 *  +---------------------+
 *  | ...                 |
 *  +---------------------+
 *  | char* argv[0]       |
 *  +---------------------+
 *  | long argc           | <------+ Program arguments
 *  +---------------------+
 *  |                     |
 *  |                     |
 *  +                     +
 *
 */
pub struct InitStack {
    /// The high address of init stack
    init_stack_top: Vaddr,
    init_stack_size: usize,
    pos: usize,
    /// Command line args
    argv: Vec<CString>,
    /// Environmental variables
    envp: Vec<CString>,
    /// Auxiliary Vector
    aux_vec: AuxVec,
}

impl InitStack {
    /// initialize user stack on base addr
    pub fn new(filename: CString, init_stack_top: Vaddr, init_stack_size: usize) -> Self {
        let argv = vec![filename];
        Self {
            init_stack_top,
            init_stack_size,
            pos: init_stack_top,
            argv,
            envp: Vec::new(),
            aux_vec: AuxVec::new(),
        }
    }

    /// This function only work for first process
    pub fn new_default_config(filename: CString) -> Self {
        let init_stack_top = INIT_STACK_BASE - PAGE_SIZE;
        let init_stack_size = INIT_STACK_SIZE;
        InitStack::new(filename, init_stack_top, init_stack_size)
    }

    /// the user stack top(high address), used to setup rsp
    pub fn user_stack_top(&self) -> Vaddr {
        let stack_top = self.pos;
        // ensure stack top is 16-bytes aligned
        debug_assert!(stack_top & !0xf == stack_top);
        stack_top
    }

    /// the user stack bottom(low address)
    const fn user_stack_bottom(&self) -> Vaddr {
        self.init_stack_top - self.init_stack_size
    }

    pub fn init(
        &mut self,
        vm_space: &VmSpace,
        elf_header_info: &ElfHeaderInfo,
    ) -> Result<(), ElfError> {
        self.map_and_zeroed(vm_space);
        self.write_zero_page(vm_space); // This page is used to store page header table
        self.write_stack_content(vm_space, elf_header_info);
        self.debug_print_stack_content(vm_space);
        Ok(())
    }

    fn map_and_zeroed(&self, vm_space: &VmSpace) {
        let vm_page_range = VmPageRange::new_range(self.user_stack_bottom()..self.user_stack_top());
        let vm_perm = InitStack::perm();
        vm_page_range.map_zeroed(vm_space, vm_perm);
    }

    /// Libc ABI requires 16-byte alignment of the stack entrypoint.
    /// Current postion of the stack is 8-byte aligned already, insert 8 byte
    /// to meet the requirement if necessary.
    fn adjust_stack_alignment(
        &mut self,
        vm_space: &VmSpace,
        envp_pointers: &Vec<u64>,
        argv_pointers: &Vec<u64>,
    ) {
        // ensure 8-byte alignment
        self.write_u64(0, vm_space);
        let auxvec_size = (self.aux_vec.table().len() + 1) * (mem::size_of::<u64>() * 2);
        let envp_pointers_size = (envp_pointers.len() + 1) * mem::size_of::<u64>();
        let argv_pointers_size = (argv_pointers.len() + 1) * mem::size_of::<u64>();
        let argc_size = mem::size_of::<u64>();
        let to_write_size = auxvec_size + envp_pointers_size + argv_pointers_size + argc_size;
        if (self.pos - to_write_size) % 16 != 0 {
            self.write_u64(0, vm_space);
        }
    }

    fn write_zero_page(&mut self, vm_space: &VmSpace) {
        self.pos -= PAGE_SIZE;
    }

    fn write_stack_content(&mut self, vm_space: &VmSpace, elf_header_info: &ElfHeaderInfo) {
        // write envp string
        let envp_pointers = self.write_envp_strings(vm_space);
        // write argv string
        let argv_pointers = self.write_argv_strings(vm_space);
        // write random value
        let random_value = generate_random_for_aux_vec();
        let random_value_pointer = self.write_bytes(&random_value, vm_space);
        self.aux_vec
            .set(AuxKey::AT_RANDOM, random_value_pointer)
            .expect("Set random value failed");
        self.aux_vec
            .set(AuxKey::AT_PAGESZ, PAGE_SIZE as _)
            .expect("Set Page Size failed");
        self.aux_vec
            .set(
                AuxKey::AT_PHDR,
                self.init_stack_top as u64 - PAGE_SIZE as u64 + elf_header_info.ph_off,
            )
            .unwrap();
        self.aux_vec
            .set(AuxKey::AT_PHNUM, elf_header_info.ph_num as u64)
            .unwrap();
        self.aux_vec
            .set(AuxKey::AT_PHENT, elf_header_info.ph_ent as u64)
            .unwrap();
        self.adjust_stack_alignment(vm_space, &envp_pointers, &argv_pointers);
        self.write_aux_vec(vm_space);
        self.write_envp_pointers(vm_space, envp_pointers);
        self.write_argv_pointers(vm_space, argv_pointers);
        // write argc
        let argc = self.argc();
        self.write_u64(argc, vm_space);
    }

    fn write_envp_strings(&mut self, vm_space: &VmSpace) -> Vec<u64> {
        let envp = self
            .envp
            .iter()
            .map(|envp| envp.clone())
            .collect::<Vec<_>>();
        let mut envp_pointers = Vec::with_capacity(envp.len());
        for envp in envp.iter() {
            let pointer = self.write_cstring(envp, vm_space);
            envp_pointers.push(pointer);
        }
        envp_pointers
    }

    fn write_argv_strings(&mut self, vm_space: &VmSpace) -> Vec<u64> {
        let argv = self
            .argv
            .iter()
            .map(|argv| argv.clone())
            .collect::<Vec<_>>();
        let mut argv_pointers = Vec::with_capacity(argv.len());
        for argv in argv.iter().rev() {
            let pointer = self.write_cstring(argv, vm_space);
            argv_pointers.push(pointer);
        }
        argv_pointers.reverse();
        argv_pointers
    }

    fn write_aux_vec(&mut self, vm_space: &VmSpace) {
        // Write NULL auxilary
        self.write_u64(0, vm_space);
        self.write_u64(AuxKey::AT_NULL as u64, vm_space);
        // Write Auxiliary vectors
        let aux_vec: Vec<_> = self
            .aux_vec
            .table()
            .iter()
            .map(|(aux_key, aux_value)| (*aux_key, *aux_value))
            .collect();
        for (aux_key, aux_value) in aux_vec.iter() {
            self.write_u64(*aux_value, vm_space);
            self.write_u64(*aux_key as u64, vm_space);
        }
    }

    fn write_envp_pointers(&mut self, vm_space: &VmSpace, mut envp_pointers: Vec<u64>) {
        // write NULL pointer
        self.write_u64(0, vm_space);
        // write envp pointers
        envp_pointers.reverse();
        for envp_pointer in envp_pointers {
            self.write_u64(envp_pointer, vm_space);
        }
    }

    fn write_argv_pointers(&mut self, vm_space: &VmSpace, mut argv_pointers: Vec<u64>) {
        // write 0
        self.write_u64(0, vm_space);
        // write argv pointers
        argv_pointers.reverse();
        for argv_pointer in argv_pointers {
            self.write_u64(argv_pointer, vm_space);
        }
    }

    /// Command line argument counter
    pub fn argc(&self) -> u64 {
        self.argv.len() as u64
    }

    /// Command linke argument start address
    pub fn argv(&self) -> u64 {
        self.user_stack_top() as u64 + 8
    }

    /// Environmental variables counter
    pub fn envc(&self) -> u64 {
        self.envp.len() as u64
    }

    /// Environmental variables pointers
    pub fn envp(&self) -> u64 {
        0
    }

    /// returns the top address of init stack.
    /// It should points to a fixed address.
    pub const fn init_stack_top(&self) -> Vaddr {
        self.init_stack_top
    }

    /// returns the u64 start address
    fn write_u64(&mut self, val: u64, vm_space: &VmSpace) -> u64 {
        let start_address = (self.pos - 8).align_down(8);
        self.pos = start_address;
        vm_space
            .write_val(start_address, &val)
            .expect("Write u64 failed");
        self.pos as u64
    }

    fn write_bytes(&mut self, bytes: &[u8], vm_space: &VmSpace) -> u64 {
        let len = bytes.len();
        self.pos -= len;
        vm_space
            .write_bytes(self.pos, bytes)
            .expect("Write String failed");
        self.pos as u64
    }

    /// returns the string start address
    /// cstring will with end null byte.
    fn write_cstring(&mut self, val: &CString, vm_space: &VmSpace) -> u64 {
        let bytes = val.as_bytes_with_nul();
        self.write_bytes(bytes, vm_space)
    }

    pub const fn perm() -> VmPerm {
        VmPerm::RWU
    }

    fn debug_print_stack_content(&self, vm_space: &VmSpace) {
        debug!("print stack content:");
        let stack_top = self.user_stack_top();
        let argc = vm_space.read_val::<u64>(stack_top).unwrap();
        debug!("argc = {}", argc);
    }
}

/// generate random [u8; 16].
/// FIXME: generate really random value. Now only return array with fixed values.
fn generate_random_for_aux_vec() -> [u8; 16] {
    let mut rand_val = [0; 16];
    for i in 0..16u8 {
        rand_val[i as usize] = 0xff - i;
    }
    rand_val
}
