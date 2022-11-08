//! Virtual Memory Address Regions (VMARs).

mod static_cap;
mod dyn_cap;
mod options;

/// Virtual Memory Address Regions (VMARs) are a type of capability that manages
/// user address spaces.
/// 
/// # Capabilities
/// 
/// As a capability, each VMAR is associated with a set of access rights, 
/// whose semantics are explained below.
/// 
/// The semantics of each access rights for VMARs are described below:
/// * The Dup right allows duplicating a VMAR and creating children out of 
/// a VMAR.
/// * The Read, Write, Exec rights allow creating memory mappings with 
/// readable, writable, and executable access permissions, respectively.
/// * The Read and Write rights allow the VMAR to be read from and written to
/// directly.
/// 
/// VMARs are implemented with two flavors of capabilities:
/// the dynamic one (`Vmar<Rights>`) and the static one (`Vmar<R: TRights>).
/// 
/// # Implementation
/// 
/// `Vmar` provides high-level APIs for address space management by wrapping 
/// around its low-level counterpart `kx_frame::vm::VmFrames`.
/// Compared with `VmFrames`,
/// `Vmar` is easier to use (by offering more powerful APIs) and 
/// harder to misuse (thanks to its nature of being capability).
/// 
pub struct Vmar<R = Rights>(Arc<Vmar_>, R);

// TODO: how page faults can be delivered to and handled by the current VMAR.

struct Vmar_ {
    inner: Mutex<Inner>,
    // The offset relative to the root VMAR
    base: Vaddr,
    parent: Option<Arc<Vmar_>>,
}

struct Inner {
    is_destroyed: bool,
    vm_space: VmSpace,
    //...
}

impl Vmar_ {
    pub fn new() -> Result<Self> {
        todo!()
    }

    pub fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        todo!()
    }

    pub fn destroy_all(&self) -> Result<()> {
        todo!()
    }

    pub fn destroy(&self, range: Range<usize>) -> Result<()> {
        todo!()
    }

    pub fn read(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        todo!()
    }

    pub fn write(&self, offset: usize, buf: &[u8]) -> Result<()> {
        todo!()
    }
}

impl<R> Vmar<R> {
    /// The base address, i.e., the offset relative to the root VMAR.
    /// 
    /// The base address of a root VMAR is zero.
    pub fn base(&self) -> Vaddr {
        self.base
    }

    fn check_rights(&self, rights: Rights) -> Result<()> {
        if self.rights.contains(rights) {
            Ok(())
        } else {
            Err(EACCESS)
        }
    }
}

bitflags! {
    /// The memory access permissions of memory mappings.
    pub struct VmPerms: u32 {
        /// Readable.
        const READ: u32     = 1 << 0;
        /// Writable.
        const WRITE: u32    = 1 << 1;
        /// Executable.
        const EXEC: u32     = 1 << 2;
    }
}

impl From<Rights> for VmPerms {
    fn from(perms: VmPerms) -> Rights {
        todo!()
    }
}