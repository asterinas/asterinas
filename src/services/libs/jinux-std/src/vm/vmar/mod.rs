//! Virtual Memory Address Regions (VMARs).

mod dyn_cap;
mod options;
mod static_cap;

use crate::rights::Rights;
use alloc::sync::Arc;
use bitflags::bitflags;
use core::ops::Range;
use jinux_frame::prelude::Result;
use jinux_frame::vm::Vaddr;
use jinux_frame::vm::VmSpace;
use jinux_frame::Error;
use spin::Mutex;

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
/// around its low-level counterpart `_frame::vm::VmFrames`.
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
        self.0.base
    }
}

bitflags! {
    /// The memory access permissions of memory mappings.
    pub struct VmPerms: u32 {
        /// Readable.
        const READ    = 1 << 0;
        /// Writable.
        const WRITE   = 1 << 1;
        /// Executable.
        const EXEC   = 1 << 2;
    }
}

impl From<Rights> for VmPerms {
    fn from(rights: Rights) -> VmPerms {
        todo!()
    }
}

impl From<VmPerms> for Rights {
    fn from(vm_perms: VmPerms) -> Rights {
        todo!()
    }
}
