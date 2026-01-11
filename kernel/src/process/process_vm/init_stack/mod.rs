// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

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
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use ostd::mm::{MAX_USERSPACE_VADDR, VmIo};

use self::aux_vec::{AuxKey, AuxVec};
use crate::{
    prelude::*,
    util::random::getrandom,
    vm::{
        perms::VmPerms,
        vmar::Vmar,
        vmo::{Vmo, VmoOptions},
    },
};

pub mod aux_vec;

/// Set the initial stack size to 8 megabytes, following the default Linux stack size limit.
pub const INIT_STACK_SIZE: usize = 8 * 1024 * 1024; // 8 MB

/// The maximum number of argument or environment strings that can be supplied to
/// the `execve` system call.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/uapi/linux/binfmts.h#L15>.
pub const MAX_NR_STRING_ARGS: usize = i32::MAX as usize;

/// The maximum size, in bytes, of a single argument or environment string
/// (`argv` / `envp`) accepted by `execve`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/uapi/linux/binfmts.h#L16>.
pub const MAX_LEN_STRING_ARG: usize = PAGE_SIZE * 32;

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
    pos: AtomicUsize,
    argv_range: SpinLock<Range<Vaddr>>,
    envp_range: SpinLock<Range<Vaddr>>,
}

impl Clone for InitStack {
    fn clone(&self) -> Self {
        Self {
            initial_top: self.initial_top,
            max_size: self.max_size,
            pos: AtomicUsize::new(self.pos.load(Ordering::Relaxed)),
            argv_range: SpinLock::new(self.argv_range.lock().clone()),
            envp_range: SpinLock::new(self.envp_range.lock().clone()),
        }
    }
}

impl InitStack {
    pub fn new() -> Self {
        let nr_pages_padding = {
            // We do not want the stack top too close to MAX_USERSPACE_VADDR.
            // So we add this fixed padding. Any small value greater than zero will do.
            const NR_FIXED_PADDING_PAGES: usize = 7;

            // Some random padding pages are added as a simple measure to
            // make the stack values of a buggy user program harder
            // to be exploited by attackers.
            let mut nr_random_padding_pages: u8 = 0;
            getrandom(nr_random_padding_pages.as_bytes_mut());

            nr_random_padding_pages as usize + NR_FIXED_PADDING_PAGES
        };
        let initial_top = MAX_USERSPACE_VADDR - PAGE_SIZE * nr_pages_padding;
        let max_size = INIT_STACK_SIZE;

        Self {
            initial_top,
            max_size,
            pos: AtomicUsize::new(initial_top),
            argv_range: SpinLock::new(0..0),
            envp_range: SpinLock::new(0..0),
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
        vmar: &Vmar,
        argv: Vec<CString>,
        envp: Vec<CString>,
        auxvec: AuxVec,
    ) -> Result<()> {
        self.set_uninitialized();

        let vmo = {
            let vmo_options = VmoOptions::new(self.max_size);
            vmo_options.alloc()?
        };
        let vmar_map_options = {
            let perms = VmPerms::READ | VmPerms::WRITE;
            let map_addr = self.initial_top - self.max_size;
            debug_assert!(map_addr.is_multiple_of(PAGE_SIZE));
            vmar.new_map(self.max_size, perms)?
                .offset(map_addr)
                .vmo(vmo.clone())
        };
        vmar_map_options.build()?;

        let writer = InitStackWriter {
            pos: &self.pos,
            vmo,
            argv,
            envp,
            auxvec,
            map_addr: self.initial_top - self.max_size,
        };
        let (argv_range, envp_range) = writer.write()?;

        *self.argv_range.lock() = argv_range;
        *self.envp_range.lock() = envp_range;

        Ok(())
    }

    /// Constructs a reader to parse the content of an `InitStack`.
    /// The `InitStack` should only be read after initialized
    pub(super) fn reader<'a>(&self, vmar: &'a Vmar) -> InitStackReader<'a> {
        debug_assert!(self.is_initialized());
        InitStackReader {
            base: self.pos(),
            vmar,
            map_addr: self.initial_top - self.max_size,
            argv_range: self.argv_range.lock().clone(),
            envp_range: self.envp_range.lock().clone(),
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
struct InitStackWriter<'a> {
    pos: &'a AtomicUsize,
    vmo: Arc<Vmo>,
    argv: Vec<CString>,
    envp: Vec<CString>,
    auxvec: AuxVec,
    /// The mapping address of the `InitStack`.
    map_addr: usize,
}

impl InitStackWriter<'_> {
    /// Writes the content to the init stack.
    ///
    /// Returns the range of argv and envp in the init stack.
    fn write(mut self) -> Result<(Range<Vaddr>, Range<Vaddr>)> {
        // FIXME: Some OSes may put the first page of executable file here
        // for interpreting elf headers.

        let argc = self.argv.len() as u64;

        // Write envp string
        let envp_end = self.pos();
        let envp_pointers = self.write_envp_strings()?;
        let envp_start = self.pos();

        // Write argv string
        let argv_end = self.pos();
        let argv_pointers = self.write_argv_strings()?;
        let argv_start = self.pos();

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

        Ok((argv_start..argv_end, envp_start..envp_end))
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
        let auxvec_size = (self.auxvec.table().len() + 1) * (size_of::<u64>() * 2);
        let envp_pointers_size = (envp_pointers.len() + 1) * size_of::<u64>();
        let argv_pointers_size = (argv_pointers.len() + 1) * size_of::<u64>();
        let argc_size = size_of::<u64>();
        let to_write_size = auxvec_size + envp_pointers_size + argv_pointers_size + argc_size;
        if !(self.pos() - to_write_size).is_multiple_of(16) {
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
        let new_pos = self.reserve_pos(size_of::<u64>(), align_of::<u64>())?;
        self.vmo.write_val(new_pos - self.map_addr, &val)?;
        Ok(new_pos as u64)
    }

    /// Writes a CString including the ending null byte to the stack.
    /// Returns the writing address
    fn write_cstring(&self, val: &CString) -> Result<u64> {
        let bytes = val.as_bytes_with_nul();
        self.write_bytes(bytes)
    }

    /// Writes bytes to the stack.
    /// Returns the writing address.
    fn write_bytes(&self, bytes: &[u8]) -> Result<u64> {
        let new_pos = self.reserve_pos(bytes.len(), align_of::<u8>())?;
        self.vmo.write_bytes(new_pos - self.map_addr, bytes)?;
        Ok(new_pos as u64)
    }

    fn reserve_pos(&self, size: usize, align: usize) -> Result<Vaddr> {
        if let Some(pos) = self.pos().checked_sub(size)
            && let new_pos = pos & !(align - 1)
            && new_pos >= self.map_addr
        {
            self.pos.store(new_pos, Ordering::Relaxed);
            return Ok(new_pos);
        }
        return_errno_with_message!(Errno::E2BIG, "Init stack overflow");
    }

    fn pos(&self) -> Vaddr {
        self.pos.load(Ordering::Relaxed)
    }
}

fn generate_random_for_aux_vec() -> [u8; 16] {
    let mut rand_val = [0; 16];
    getrandom(&mut rand_val);
    rand_val
}

/// A reader to parse the content of an `InitStack`.
pub struct InitStackReader<'a> {
    base: Vaddr,
    vmar: &'a Vmar,
    /// The mapping address of the `InitStack`.
    map_addr: usize,
    argv_range: Range<Vaddr>,
    envp_range: Range<Vaddr>,
}

impl InitStackReader<'_> {
    /// Reads argc from the process init stack.
    pub fn argc(&self) -> Result<u64> {
        let mut buffer = [0u8; 8];

        self.vmar.read_remote(
            self.init_stack_bottom(),
            &mut VmWriter::from(&mut buffer[..]).to_fallible(),
        )?;

        let argc = u64::from_ne_bytes(buffer);
        if argc > MAX_NR_STRING_ARGS as u64 {
            return_errno_with_message!(Errno::EINVAL, "argc is corrupted");
        }

        Ok(argc)
    }

    /// Reads argv at the `offset` from the process init stack.
    pub fn argv(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if offset >= self.argv_range.end - self.argv_range.start {
            return Ok(0);
        }

        let read_at = self.argv_range.start + offset;
        writer.limit(self.argv_range.end - read_at);
        let bytes_read = self.vmar.read_remote(read_at, writer)?;

        Ok(bytes_read)
    }

    /// Reads envp at the `offset` from the process init stack.
    pub fn envp(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if offset >= self.envp_range.end - self.envp_range.start {
            return Ok(0);
        }

        let read_at = self.envp_range.start + offset;
        writer.limit(self.envp_range.end - read_at);
        let bytes_read = self.vmar.read_remote(read_at, writer)?;

        Ok(bytes_read)
    }

    /// Returns the bottom address of the init stack (lowest address).
    pub const fn init_stack_bottom(&self) -> Vaddr {
        self.base
    }
}
