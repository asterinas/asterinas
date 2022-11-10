//! Virtual Memory Objects (VMOs).

use core::ops::Range;

use kxos_frame::{prelude::Result, vm::Paddr, Error};
use crate::rights::Rights;
use alloc::sync::Arc;
use bitflags::bitflags;

mod static_cap;
mod dyn_cap;
mod options;
mod pager;

pub use options::{VmoOptions, VmoChildOptions};
pub use pager::Pager;
use spin::Mutex;



/// Virtual Memory Objects (VMOs) are a type of capability that represents a 
/// range of memory pages.
/// 
/// # Features
/// 
/// * **I/O interface.** A VMO provides read and write methods to access the
/// memory pages that it contain.
/// * **On-demand paging.** The memory pages of a VMO (except for _contiguous_ 
/// VMOs) are allocated lazily when the page is first accessed.
/// * **Tree structure.** Given a VMO, one can create a child VMO from it.
/// The child VMO can only access a subset of the parent's memory,
/// which is a good thing for the perspective of access control.
/// * **Copy-on-write (COW).** A child VMO may be created with COW semantics,
/// which prevents any writes on the child from affecting the parent
/// by duplicating memory pages only upon the first writes.
/// * **Access control.** As capabilities, VMOs restrict the 
/// accessible range of memory and the allowed I/O operations.
/// * **Device driver support.** If specified upon creation, VMOs will be
/// backed by physically contiguous memory pages starting at a target address.
/// * **File system support.** By default, a VMO's memory pages are initially
/// all zeros. But if a VMO is attached to a pager (`Pager`) upon creation, 
/// then its memory pages will be populated by the pager.
/// With this pager mechanism, file systems can easily implement page caches
/// with VMOs by attaching the VMOs to pagers backed by inodes.
/// 
/// # Capabilities
/// 
/// As a capability, each VMO is associated with a set of access rights, 
/// whose semantics are explained below.
/// 
/// * The Dup right allows duplicating a VMO and creating children out of 
/// a VMO.
/// * The Read, Write, Exec rights allow creating memory mappings with 
/// readable, writable, and executable access permissions, respectively.
/// * The Read and Write rights allow the VMO to be read from and written to
/// directly.
/// * The Write right allows resizing a resizable VMO.
/// 
/// VMOs are implemented with two flavors of capabilities:
/// the dynamic one (`Vmo<Rights>`) and the static one (`Vmo<R: TRights>).
/// 
/// # Examples
/// 
/// For creating root VMOs, see `VmoOptions`.`
/// 
/// For creating child VMOs, see `VmoChildOptions`.
/// 
/// # Implementation
/// 
/// `Vmo` provides high-level APIs for address space management by wrapping 
/// around its low-level counterpart `kx_frame::vm::VmFrames`.
/// Compared with `VmFrames`,
/// `Vmo` is easier to use (by offering more powerful APIs) and 
/// harder to misuse (thanks to its nature of being capability).
/// 
pub struct Vmo<R>(Arc<Vmo_>, R);

bitflags! {
    /// VMO flags.
    pub struct VmoFlags: u32 {
        /// Set this flag if a VMO is resizable.
        const RESIZABLE  = 1 << 0;
        /// Set this flags if a VMO is backed by physically contiguous memory 
        /// pages.
        /// 
        /// To ensure the memory pages to be contiguous, these pages
        /// are allocated upon the creation of the VMO, rather than on demands. 
        const CONTIGUOUS = 1 << 1;
        /// Set this flag if a VMO is backed by memory pages that supports 
        /// Direct Memory Access (DMA) by devices.
        const DMA        = 1 << 2;
    }
}

struct Vmo_ {
    flags: VmoFlags,
    inner: Mutex<VmoInner>, 
    parent: Option<Arc<Vmo_>>,
}

struct VmoInner {
    //...
}

impl Vmo_ {
    pub fn commit_page(&self, offset: usize) -> Result<()> {
        todo!()
    }

    pub fn decommit_page(&self, offset: usize) -> Result<()> {
        todo!()
    }

    pub fn commit(&self, range: Range<usize>) -> Result<()> {
        todo!()
    }

    pub fn decommit(&self, range: Range<usize>) -> Result<()> {
        todo!()
    }

    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        todo!()
    }

    pub fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        todo!()
    }

    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        todo!()
    }

    pub fn size(&self) -> usize {
        todo!()
    }

    pub fn resize(&self, new_size: usize) -> Result<()> {
        todo!()
    }

    pub fn paddr(&self) -> Option<Paddr> {
        todo!()
    }

    pub fn flags(&self) -> VmoFlags {
        todo!()
    }
}


impl<R> Vmo<R> {
    /// Returns the size (in bytes) of a VMO.
    pub fn size(&self) -> usize {
        self.0.size()
    }

    /// Returns the starting physical address of a VMO, if it is contiguous.
    /// Otherwise, returns none.
    pub fn paddr(&self) -> Option<Paddr> {
        self.0.paddr()
    }

    /// Returns the flags of a VMO.
    pub fn flags(&self) -> VmoFlags {
        self.0.flags()
    }

}