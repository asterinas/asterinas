// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

//! The init stack for the process.
//! The init stack is used to store the `argv` and `envp` and auxiliary vectors.
//! We can read `argv` and `envp` of a process from the init stack.
//! Usually, the lowest address of init stack is
//! the highest address of the user stack of the first thread.
//!
//! However, the init stack will be mapped to user space
//! and the user process can write the content of init stack,
//! so the content reading from init stack may not be the same as the process init status.
//!

use core::{
    mem,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use aster_rights::Full;
use ostd::{
    mm::{VmIo, MAX_USERSPACE_VADDR},
    task::Task,
};

use self::aux_vec::{AuxKey, AuxVec};
use crate::{
    get_current_userspace,
    prelude::*,
    util::random::getrandom,
    vm::{
        perms::VmPerms,
        vmar::Vmar,
        vmo::{Vmo, VmoOptions, VmoRightsOp},
    },
};

pub mod aux_vec;

/// Set the initial stack size to 8 megabytes, following the default Linux stack size limit.
pub const INIT_STACK_SIZE: usize = 8 * 1024 * 1024; // 8 MB

/// The max number of arguments that can be used to creating a new process.
pub const MAX_ARGV_NUMBER: usize = 128;
/// The max number of environmental variables that can be used to creating a new process.
pub const MAX_ENVP_NUMBER: usize = 128;
/// The max length of each argument to create a new process.
pub const MAX_ARG_LEN: usize = 2048;
/// The max length of each environmental variable (the total length of key-value pair) to create a new process.
pub const MAX_ENV_LEN: usize = 128;

/*
 * Illustration of the virtual memory space containing the processes' init stack:
 *
 *  (high address)
 *  +---------------------+ <------+ Highest address
 *  |                     |          Random stack paddings
 *  +---------------------+ <------+ The base of stack (stack grows down)
 *  |                     |
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
 *  +---------------------+
 *  |                     |
 *  +---------------------+ <------+ User stack default rlimit
 *  (low address)
 */

/// The initial portion of the main stack of a process.
pub struct InitStack {
    /// The initial highest address.
    /// The stack grows down from this address
    initial_top: Vaddr,
    /// The max allowed stack size
    max_size: usize,
    /// The current stack pointer.
    /// Before initialized, `pos` points to the `initial_top`,
    /// After initialized, `pos` points to the user stack pointer(rsp)
    /// of the process.
    pos: Arc<AtomicUsize>,
}

impl Clone for InitStack {
    fn clone(&self) -> Self {
        Self {
            initial_top: self.initial_top,
            max_size: self.max_size,
            pos: Arc::new(AtomicUsize::new(self.pos.load(Ordering::Relaxed))),
        }
    }
}

impl InitStack {
    pub(super) fn new() -> Self {
        let nr_pages_padding = {
            let mut random_nr_pages_padding: u8 = 0;
            getrandom(random_nr_pages_padding.as_bytes_mut()).unwrap();
            random_nr_pages_padding as usize
        };
        let initial_top = MAX_USERSPACE_VADDR - PAGE_SIZE * nr_pages_padding;
        let max_size = INIT_STACK_SIZE;

        Self {
            initial_top,
            max_size,
            pos: Arc::new(AtomicUsize::new(initial_top)),
        }
    }

    /// Returns the user stack top(highest address), used to setup rsp.
    ///
    /// This method should only be called after the stack is initialized.
    pub fn user_stack_top(&self) -> Vaddr {
        let stack_top = self.pos();
        debug_assert!(self.is_initialized());

        stack_top
    }

    /// Maps the VMO of the init stack and constructs a writer to initialize its content.
    pub(super) fn map_and_write(
        &self,
        root_vmar: &Vmar<Full>,
        argv: Vec<CString>,
        envp: Vec<CString>,
        auxvec: AuxVec,
    ) -> Result<()> {
        self.set_uninitialized();

        let vmo = {
            let vmo_options = VmoOptions::<Full>::new(self.max_size);
            vmo_options.alloc()?
        };
        let vmar_map_options = {
            let perms = VmPerms::READ | VmPerms::WRITE;
            let map_addr = self.initial_top - self.max_size;
            debug_assert!(map_addr % PAGE_SIZE == 0);
            root_vmar
                .new_map(self.max_size, perms)?
                .offset(map_addr)
                .vmo(vmo.dup().to_dyn())
        };
        vmar_map_options.build()?;

        let writer = InitStackWriter {
            pos: self.pos.clone(),
            vmo,
            argv,
            envp,
            auxvec,
            map_addr: self.initial_top - self.max_size,
        };
        writer.write()
    }

    /// Constructs a reader to parse the content of an `InitStack`.
    /// The `InitStack` should only be read after initialized
    pub(super) fn reader(&self) -> InitStackReader {
        debug_assert!(self.is_initialized());
        InitStackReader {
            base: self.pos(),
            map_addr: self.initial_top - self.max_size,
        }
    }

    fn is_initialized(&self) -> bool {
        self.pos() != self.initial_top
    }

    fn set_uninitialized(&self) {
        self.pos.store(self.initial_top, Ordering::Relaxed);
    }

    fn pos(&self) -> Vaddr {
        self.pos.load(Ordering::Relaxed)
    }
}

/// A writer to initialize the content of an `InitStack`.
struct InitStackWriter {
    pos: Arc<AtomicUsize>,
    vmo: Vmo<Full>,
    argv: Vec<CString>,
    envp: Vec<CString>,
    auxvec: AuxVec,
    /// The mapping address of the `InitStack`.
    map_addr: usize,
}

impl InitStackWriter {
    fn write(mut self) -> Result<()> {
        // FIXME: Some OSes may put the first page of executable file here
        // for interpreting elf headers.

        let argc = self.argv.len() as u64;

        // Write envp string
        let envp_pointers = self.write_envp_strings()?;
        // Write argv string
        let argv_pointers = self.write_argv_strings()?;
        // Generate random values for auxvec
        let random_value_pointer = {
            let random_value = generate_random_for_aux_vec();
            self.write_bytes(&random_value)?
        };
        self.auxvec.set(AuxKey::AT_RANDOM, random_value_pointer)?;

        self.adjust_stack_alignment(&envp_pointers, &argv_pointers)?;
        self.write_aux_vec()?;
        self.write_envp_pointers(envp_pointers)?;
        self.write_argv_pointers(argv_pointers)?;

        // write argc
        self.write_u64(argc)?;

        // Ensure stack top is 16-bytes aligned
        debug_assert_eq!(self.pos() & !0xf, self.pos());

        Ok(())
    }

    fn write_envp_strings(&self) -> Result<Vec<u64>> {
        let mut envp_pointers = Vec::with_capacity(self.envp.len());
        for envp in self.envp.iter() {
            let pointer = self.write_cstring(envp)?;
            envp_pointers.push(pointer);
        }
        Ok(envp_pointers)
    }

    fn write_argv_strings(&self) -> Result<Vec<u64>> {
        let mut argv_pointers = Vec::with_capacity(self.argv.len());
        for argv in self.argv.iter().rev() {
            let pointer = self.write_cstring(argv)?;
            debug!("argv address = 0x{:x}", pointer);
            argv_pointers.push(pointer);
        }
        argv_pointers.reverse();
        Ok(argv_pointers)
    }

    /// Libc ABI requires 16-byte alignment of the stack entrypoint.
    /// Current position of the stack is 8-byte aligned already, insert 8 byte
    /// to meet the requirement if necessary.
    fn adjust_stack_alignment(&self, envp_pointers: &[u64], argv_pointers: &[u64]) -> Result<()> {
        // Ensure 8-byte alignment
        self.write_u64(0)?;
        let auxvec_size = (self.auxvec.table().len() + 1) * (mem::size_of::<u64>() * 2);
        let envp_pointers_size = (envp_pointers.len() + 1) * mem::size_of::<u64>();
        let argv_pointers_size = (argv_pointers.len() + 1) * mem::size_of::<u64>();
        let argc_size = mem::size_of::<u64>();
        let to_write_size = auxvec_size + envp_pointers_size + argv_pointers_size + argc_size;
        if (self.pos() - to_write_size) % 16 != 0 {
            self.write_u64(0)?;
        }
        Ok(())
    }

    fn write_aux_vec(&self) -> Result<()> {
        // Write NULL auxiliary
        self.write_u64(0)?;
        self.write_u64(AuxKey::AT_NULL as u64)?;
        // Write Auxiliary vectors
        let aux_vec: Vec<_> = self
            .auxvec
            .table()
            .iter()
            .map(|(aux_key, aux_value)| (*aux_key, *aux_value))
            .collect();
        for (aux_key, aux_value) in aux_vec.iter() {
            self.write_u64(*aux_value)?;
            self.write_u64(*aux_key as u64)?;
        }
        Ok(())
    }

    fn write_envp_pointers(&self, mut envp_pointers: Vec<u64>) -> Result<()> {
        // write NULL pointer
        self.write_u64(0)?;
        // write envp pointers
        envp_pointers.reverse();
        for envp_pointer in envp_pointers {
            self.write_u64(envp_pointer)?;
        }
        Ok(())
    }

    fn write_argv_pointers(&self, mut argv_pointers: Vec<u64>) -> Result<()> {
        // write 0
        self.write_u64(0)?;
        // write argv pointers
        argv_pointers.reverse();
        for argv_pointer in argv_pointers {
            self.write_u64(argv_pointer)?;
        }
        Ok(())
    }

    /// Writes u64 to the stack.
    /// Returns the writing address
    fn write_u64(&self, val: u64) -> Result<u64> {
        let start_address = (self.pos() - 8).align_down(8);
        self.pos.store(start_address, Ordering::Relaxed);
        self.vmo.write_val(start_address - self.map_addr, &val)?;
        Ok(self.pos() as u64)
    }

    /// Writes a CString including the ending null byte to the stack.
    /// Returns the writing address
    fn write_cstring(&self, val: &CString) -> Result<u64> {
        let bytes = val.as_bytes_with_nul();
        self.write_bytes(bytes)
    }

    /// Writes u64 to the stack.
    /// Returns the writing address.
    fn write_bytes(&self, bytes: &[u8]) -> Result<u64> {
        let len = bytes.len();
        self.pos.fetch_sub(len, Ordering::Relaxed);
        let pos = self.pos();
        self.vmo.write_bytes(pos - self.map_addr, bytes)?;
        Ok(pos as u64)
    }

    fn pos(&self) -> Vaddr {
        self.pos.load(Ordering::Relaxed)
    }
}

fn generate_random_for_aux_vec() -> [u8; 16] {
    let mut rand_val = [0; 16];
    getrandom(&mut rand_val).unwrap();
    rand_val
}

/// A reader to parse the content of an `InitStack`.
pub struct InitStackReader {
    base: Vaddr,
    /// The mapping address of the `InitStack`.
    map_addr: usize,
}

impl InitStackReader {
    /// Reads argc from the process init stack
    pub fn argc(&self) -> Result<u64> {
        let stack_base = self.init_stack_bottom();
        get_current_userspace!().read_val(stack_base)
    }

    /// Reads argv from the process init stack
    pub fn argv(&self) -> Result<Vec<CString>> {
        let argc = self.argc()? as usize;
        // The reading offset in the initial stack is:
        // the initial stack bottom address + the size of `argc` in memory
        let read_offset = self.init_stack_bottom() + size_of::<usize>();

        let mut argv = Vec::with_capacity(argc);

        let current_task = Task::current().unwrap();
        let user_space = CurrentUserSpace::new(&current_task);

        let mut argv_reader = user_space.reader(read_offset, argc * size_of::<usize>())?;
        for _ in 0..argc {
            let arg = {
                let arg_ptr = argv_reader.read_val::<Vaddr>()?;
                user_space.read_cstring(arg_ptr, MAX_ARG_LEN)?
            };
            argv.push(arg);
        }

        Ok(argv)
    }

    /// Reads envp from the process
    pub fn envp(&self) -> Result<Vec<CString>> {
        let argc = self.argc()? as usize;
        // The reading offset in the initial stack is:
        // the initial stack bottom address
        // + the size of argc(8)
        // + the size of arg pointer(8) * the number of arg(argc)
        // + the size of null pointer(8)
        let read_offset = self.init_stack_bottom()
            + size_of::<usize>()
            + size_of::<usize>() * argc
            + size_of::<usize>();

        let mut envp = Vec::new();

        let current_task = Task::current().unwrap();
        let user_space = CurrentUserSpace::new(&current_task);

        let mut envp_reader = user_space.reader(read_offset, MAX_ENVP_NUMBER)?;
        for _ in 0..MAX_ENVP_NUMBER {
            let envp_ptr = envp_reader.read_val::<Vaddr>()?;

            if envp_ptr == 0 {
                break;
            }

            let env = user_space.read_cstring(envp_ptr, MAX_ENV_LEN)?;
            envp.push(env);
        }

        Ok(envp)
    }

    /// Returns the bottom address of the init stack (lowest address).
    pub const fn init_stack_bottom(&self) -> Vaddr {
        self.base
    }
}
