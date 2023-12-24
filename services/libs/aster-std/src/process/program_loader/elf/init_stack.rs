//! This module defines the process initial stack.
//! The process initial stack, contains arguments, environmental variables and auxiliary vectors
//! The data layout of init stack can be seen in Figure 3.9 in https://uclibc.org/docs/psABI-x86_64.pdf

use crate::vm::perms::VmPerms;
use crate::{
    prelude::*,
    vm::{vmar::Vmar, vmo::VmoOptions},
};
use align_ext::AlignExt;
use aster_frame::vm::{VmIo, VmPerm};
use aster_rights::{Full, Rights};
use core::mem;

use super::aux_vec::{AuxKey, AuxVec};
use super::elf_file::Elf;
use super::load_elf::LdsoLoadInfo;

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
}

impl InitStack {
    /// initialize user stack on base addr
    pub fn new(
        init_stack_top: Vaddr,
        init_stack_size: usize,
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Self {
        Self {
            init_stack_top,
            init_stack_size,
            pos: init_stack_top,
            argv,
            envp,
        }
    }

    /// This function only work for first process
    pub fn new_default_config(argv: Vec<CString>, envp: Vec<CString>) -> Self {
        let init_stack_top = INIT_STACK_BASE - PAGE_SIZE;
        let init_stack_size = INIT_STACK_SIZE;
        InitStack::new(init_stack_top, init_stack_size, argv, envp)
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
        root_vmar: &Vmar<Full>,
        elf: &Elf,
        ldso_load_info: &Option<LdsoLoadInfo>,
        aux_vec: &mut AuxVec,
    ) -> Result<()> {
        self.map_and_zeroed(root_vmar)?;
        self.write_stack_content(root_vmar, elf, ldso_load_info, aux_vec)?;
        self.debug_print_stack_content(root_vmar);
        Ok(())
    }

    fn map_and_zeroed(&self, root_vmar: &Vmar<Full>) -> Result<()> {
        let vmo_options = VmoOptions::<Rights>::new(self.init_stack_size);
        let vmo = vmo_options.alloc()?;
        vmo.clear(0..vmo.size())?;
        let perms = VmPerms::READ | VmPerms::WRITE;
        let vmar_map_options = root_vmar
            .new_map(vmo, perms)?
            .offset(self.user_stack_bottom());
        vmar_map_options.build().unwrap();
        Ok(())
    }

    /// Libc ABI requires 16-byte alignment of the stack entrypoint.
    /// Current postion of the stack is 8-byte aligned already, insert 8 byte
    /// to meet the requirement if necessary.
    fn adjust_stack_alignment(
        &mut self,
        root_vmar: &Vmar<Full>,
        envp_pointers: &[u64],
        argv_pointers: &[u64],
        aux_vec: &AuxVec,
    ) -> Result<()> {
        // ensure 8-byte alignment
        self.write_u64(0, root_vmar)?;
        let auxvec_size = (aux_vec.table().len() + 1) * (mem::size_of::<u64>() * 2);
        let envp_pointers_size = (envp_pointers.len() + 1) * mem::size_of::<u64>();
        let argv_pointers_size = (argv_pointers.len() + 1) * mem::size_of::<u64>();
        let argc_size = mem::size_of::<u64>();
        let to_write_size = auxvec_size + envp_pointers_size + argv_pointers_size + argc_size;
        if (self.pos - to_write_size) % 16 != 0 {
            self.write_u64(0, root_vmar)?;
        }
        Ok(())
    }

    fn write_stack_content(
        &mut self,
        root_vmar: &Vmar<Full>,
        elf: &Elf,
        ldso_load_info: &Option<LdsoLoadInfo>,
        aux_vec: &mut AuxVec,
    ) -> Result<()> {
        // write a zero page. When a user program tries to read a cstring(like argv) from init stack,
        // it will typically read 4096 bytes and then find the first '\0' in the buffer
        // (we now read 128 bytes, which is set by MAX_FILENAME_LEN).
        // If we don't have this zero page, the read may go into guard page,
        // which will cause unrecoverable page fault(The guard page is not backed up by any vmo).
        // So we add a zero page here, to ensure the read will not go into guard page.
        // FIXME: Some other OSes put the first page of excutable file here.
        self.write_bytes(&[0u8; PAGE_SIZE], root_vmar)?;
        // write envp string
        let envp_pointers = self.write_envp_strings(root_vmar)?;
        // write argv string
        let argv_pointers = self.write_argv_strings(root_vmar)?;
        // write random value
        let random_value = generate_random_for_aux_vec();
        let random_value_pointer = self.write_bytes(&random_value, root_vmar)?;
        aux_vec.set(AuxKey::AT_RANDOM, random_value_pointer)?;
        if let Some(ldso_load_info) = ldso_load_info {
            let ldso_base = ldso_load_info.base_addr();
            aux_vec.set(AuxKey::AT_BASE, ldso_base as u64)?;
        }
        self.adjust_stack_alignment(root_vmar, &envp_pointers, &argv_pointers, aux_vec)?;
        self.write_aux_vec(root_vmar, aux_vec)?;
        self.write_envp_pointers(root_vmar, envp_pointers)?;
        self.write_argv_pointers(root_vmar, argv_pointers)?;
        // write argc
        let argc = self.argc();
        self.write_u64(argc, root_vmar)?;
        Ok(())
    }

    fn write_envp_strings(&mut self, root_vmar: &Vmar<Full>) -> Result<Vec<u64>> {
        let envp = self.envp.to_vec();
        let mut envp_pointers = Vec::with_capacity(envp.len());
        for envp in envp.iter() {
            let pointer = self.write_cstring(envp, root_vmar)?;
            envp_pointers.push(pointer);
        }
        Ok(envp_pointers)
    }

    fn write_argv_strings(&mut self, root_vmar: &Vmar<Full>) -> Result<Vec<u64>> {
        let argv = self.argv.to_vec();
        let mut argv_pointers = Vec::with_capacity(argv.len());
        for argv in argv.iter().rev() {
            let pointer = self.write_cstring(argv, root_vmar)?;
            debug!("argv address = 0x{:x}", pointer);
            argv_pointers.push(pointer);
        }
        argv_pointers.reverse();
        Ok(argv_pointers)
    }

    fn write_aux_vec(&mut self, root_vmar: &Vmar<Full>, aux_vec: &AuxVec) -> Result<()> {
        // Write NULL auxilary
        self.write_u64(0, root_vmar)?;
        self.write_u64(AuxKey::AT_NULL as u64, root_vmar)?;
        // Write Auxiliary vectors
        let aux_vec: Vec<_> = aux_vec
            .table()
            .iter()
            .map(|(aux_key, aux_value)| (*aux_key, *aux_value))
            .collect();
        for (aux_key, aux_value) in aux_vec.iter() {
            self.write_u64(*aux_value, root_vmar)?;
            self.write_u64(*aux_key as u64, root_vmar)?;
        }
        Ok(())
    }

    fn write_envp_pointers(
        &mut self,
        root_vmar: &Vmar<Full>,
        mut envp_pointers: Vec<u64>,
    ) -> Result<()> {
        // write NULL pointer
        self.write_u64(0, root_vmar)?;
        // write envp pointers
        envp_pointers.reverse();
        for envp_pointer in envp_pointers {
            self.write_u64(envp_pointer, root_vmar)?;
        }
        Ok(())
    }

    fn write_argv_pointers(
        &mut self,
        root_vmar: &Vmar<Full>,
        mut argv_pointers: Vec<u64>,
    ) -> Result<()> {
        // write 0
        self.write_u64(0, root_vmar)?;
        // write argv pointers
        argv_pointers.reverse();
        for argv_pointer in argv_pointers {
            self.write_u64(argv_pointer, root_vmar)?;
        }
        Ok(())
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
    fn write_u64(&mut self, val: u64, root_vmar: &Vmar<Full>) -> Result<u64> {
        let start_address = (self.pos - 8).align_down(8);
        self.pos = start_address;
        root_vmar.write_val(start_address, &val)?;
        Ok(self.pos as u64)
    }

    fn write_bytes(&mut self, bytes: &[u8], root_vmar: &Vmar<Full>) -> Result<u64> {
        let len = bytes.len();
        self.pos -= len;
        root_vmar.write_bytes(self.pos, bytes)?;
        Ok(self.pos as u64)
    }

    /// returns the string start address
    /// cstring will with end null byte.
    fn write_cstring(&mut self, val: &CString, root_vmar: &Vmar<Full>) -> Result<u64> {
        let bytes = val.as_bytes_with_nul();
        self.write_bytes(bytes, root_vmar)
    }

    pub const fn perm() -> VmPerm {
        VmPerm::RWU
    }

    fn debug_print_stack_content(&self, root_vmar: &Vmar<Full>) {
        debug!("print stack content:");
        let stack_top = self.user_stack_top();
        let argc = root_vmar.read_val::<u64>(stack_top).unwrap();
        debug!("argc = {}", argc);
    }
}

pub fn init_aux_vec(elf: &Elf, elf_map_addr: Vaddr, vdso_text_base: Vaddr) -> Result<AuxVec> {
    let mut aux_vec = AuxVec::new();
    aux_vec.set(AuxKey::AT_PAGESZ, PAGE_SIZE as _)?;
    let ph_addr = if elf.is_shared_object() {
        elf.ph_addr()? + elf_map_addr
    } else {
        elf.ph_addr()?
    };
    aux_vec.set(AuxKey::AT_PHDR, ph_addr as u64)?;
    aux_vec.set(AuxKey::AT_PHNUM, elf.ph_count() as u64)?;
    aux_vec.set(AuxKey::AT_PHENT, elf.ph_ent() as u64)?;
    let elf_entry = if elf.is_shared_object() {
        let base_load_offset = elf.base_load_address_offset();
        elf.entry_point() + elf_map_addr - base_load_offset as usize
    } else {
        elf.entry_point()
    };
    aux_vec.set(AuxKey::AT_ENTRY, elf_entry as u64)?;
    aux_vec.set(AuxKey::AT_SYSINFO_EHDR, vdso_text_base as u64)?;
    Ok(aux_vec)
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
