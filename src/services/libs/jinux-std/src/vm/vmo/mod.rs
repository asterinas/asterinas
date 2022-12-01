//! Virtual Memory Objects (VMOs).

use core::ops::Range;

use crate::rights::Rights;
use alloc::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    sync::Weak,
};
use bitflags::bitflags;
use jinux_frame::{
    config::PAGE_SIZE,
    prelude::Result,
    vm::{Paddr, Vaddr, VmAllocOptions, VmFrame, VmFrameVec, VmIo, VmMapOptions, VmPerm, VmSpace},
    Error,
};

mod dyn_cap;
mod options;
mod pager;
mod static_cap;

pub use options::{VmoChildOptions, VmoOptions};
pub use pager::Pager;
use spin::Mutex;

use super::vmar::Vmar;

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
/// around its low-level counterpart `jinux_frame::vm::VmFrames`.
/// Compared with `VmFrames`,
/// `Vmo` is easier to use (by offering more powerful APIs) and
/// harder to misuse (thanks to its nature of being capability).
///
pub struct Vmo<R = Rights>(Arc<Vmo_>, R);

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

pub enum VmoType {
    /// This vmo_ is created as a copy on write child
    CopyOnWriteChild,
    /// This vmo_ is created as a slice child
    SliceChild,
    /// This vmo_ is not created as a child of a parent vmo
    NotChild,
}

struct Vmo_ {
    /// Flags
    flags: VmoFlags,
    /// VmoInner
    inner: Mutex<VmoInner>,
    /// Parent Vmo
    parent: Weak<Vmo_>,
    /// paddr
    paddr: Option<Paddr>,
    /// vmo type
    vmo_type: VmoType,
}

struct VmoInner {
    /// The backup pager
    pager: Option<Arc<dyn Pager>>,
    /// size, in bytes
    size: usize,
    /// The mapped to vmar if mapped
    mapped_to_vmar: Weak<Vmar>,
    /// The base addr in vmspace if self is mapped. Otherwise this field is useless
    mapped_to_addr: Vaddr,
    /// The pages already mapped. The key is the page index.
    mapped_pages: BTreeSet<usize>,
    /// The perm of each page. This map is filled when first time map vmo to vmar
    page_perms: BTreeMap<usize, VmPerm>,
    /// The pages committed but not mapped to Vmar. The key is the page index, the value is the backup frame.
    unmapped_pages: BTreeMap<usize, VmFrameVec>,
    /// The pages from the parent that current vmo can access. The pages can only be inserted when create childs vmo.
    /// The key is the page index in current vmo, and the value is the page index in parent vmo.
    inherited_pages: BTreeMap<usize, usize>,
    // Pages should be filled with zeros when committed. When create COW child, the pages exceed the range of parent vmo
    // should be in this set. According to the on demand requirement, when read or write these pages for the first time,
    // we should commit these pages and zeroed these pages.
    // pages_should_fill_zeros: BTreeSet<usize>,
}

impl Vmo_ {
    pub fn commit_page(&self, offset: usize) -> Result<()> {
        // assert!(offset % PAGE_SIZE == 0);
        let page_idx = offset / PAGE_SIZE;
        let is_mapped = self.is_mapped();
        let mut inner = self.inner.lock();
        if is_mapped {
            if inner.mapped_pages.contains(&page_idx) {
                return Ok(());
            }
        }

        if !inner.unmapped_pages.contains_key(&offset) {
            let frames = match &inner.pager {
                None => {
                    let vm_alloc_option = VmAllocOptions::new(1);
                    let frames = VmFrameVec::allocate(&vm_alloc_option)?;
                    frames.iter().for_each(|frame| frame.zero());
                    frames
                }
                Some(pager) => {
                    let frame = pager.commit_page(offset)?;
                    VmFrameVec::from_one_frame(frame)
                }
            };
            if is_mapped {
                // We hold the lock inside inner, so we cannot call vm_space function here
                let vm_space = inner.mapped_to_vmar.upgrade().unwrap().vm_space();
                let mapped_to_addr = inner.mapped_to_addr + page_idx * PAGE_SIZE;
                let mut vm_map_options = VmMapOptions::new();
                let vm_perm = inner.page_perms.get(&page_idx).unwrap().clone();
                vm_map_options.perm(vm_perm).addr(Some(mapped_to_addr));
                vm_space.map(frames, &vm_map_options)?;
            } else {
                inner.unmapped_pages.insert(page_idx, frames);
            }
        }
        Ok(())
    }

    pub fn decommit_page(&self, offset: usize) -> Result<()> {
        // assert!(offset % PAGE_SIZE == 0);
        let page_idx = offset / PAGE_SIZE;
        let mut inner = self.inner.lock();
        if inner.mapped_pages.contains(&page_idx) {
            // We hold the lock inside inner, so we cannot call vm_space function here
            let vm_space = inner.mapped_to_vmar.upgrade().unwrap().vm_space();
            let mapped_addr = inner.mapped_to_addr + page_idx * PAGE_SIZE;
            vm_space.unmap(&(mapped_addr..mapped_addr + PAGE_SIZE))?;
            inner.mapped_pages.remove(&page_idx);
            if let Some(pager) = &inner.pager {
                pager.decommit_page(offset)?;
            }
        } else if inner.unmapped_pages.contains_key(&page_idx) {
            inner.unmapped_pages.remove(&page_idx);
            if let Some(pager) = &inner.pager {
                pager.decommit_page(offset)?;
            }
        }
        Ok(())
    }

    pub fn commit(&self, range: Range<usize>) -> Result<()> {
        assert!(range.start % PAGE_SIZE == 0);
        assert!(range.end % PAGE_SIZE == 0);
        let start_page_idx = range.start / PAGE_SIZE;
        let end_page_idx = range.end / PAGE_SIZE;
        for page_idx in start_page_idx..end_page_idx {
            let offset = page_idx * PAGE_SIZE;
            self.commit_page(offset)?;
        }

        Ok(())
    }

    pub fn decommit(&self, range: Range<usize>) -> Result<()> {
        // assert!(range.start % PAGE_SIZE == 0);
        // assert!(range.end % PAGE_SIZE == 0);
        let start_page_idx = range.start / PAGE_SIZE;
        let end_page_idx = range.end / PAGE_SIZE;
        for page_idx in start_page_idx..end_page_idx {
            let offset = page_idx * PAGE_SIZE;
            self.decommit_page(offset)?;
        }
        Ok(())
    }

    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let read_len = buf.len();
        debug_assert!(offset + read_len <= self.size());
        if offset + read_len > self.size() {
            return Err(Error::InvalidArgs);
        }

        let first_page_idx = offset / PAGE_SIZE;
        let last_page_idx = (offset + read_len - 1) / PAGE_SIZE;
        let mut buf_read_offset = 0;
        // read one page at a time
        for page_idx in first_page_idx..=last_page_idx {
            let page_offset = if page_idx == first_page_idx {
                offset - first_page_idx * PAGE_SIZE
            } else {
                0
            };
            let page_remain_len = PAGE_SIZE - page_offset;
            let buf_remain_len = read_len - buf_read_offset;
            let read_len_in_page = page_remain_len.min(buf_remain_len);
            if read_len_in_page == 0 {
                break;
            }
            let read_buf = &mut buf[buf_read_offset..(buf_read_offset + read_len_in_page)];
            buf_read_offset += read_len_in_page;
            self.read_bytes_in_page(page_idx, page_offset, read_buf)?;
        }
        Ok(())
    }

    /// read bytes to buf. The read content are ensured on same page. if the page is not committed or mapped,
    /// this func will commit or map this page
    fn read_bytes_in_page(&self, page_idx: usize, offset: usize, buf: &mut [u8]) -> Result<()> {
        // First read from pages in parent
        if let Some(parent_page_idx) = self.inner.lock().inherited_pages.get(&page_idx) {
            let parent_vmo = self.parent.upgrade().unwrap();
            let parent_read_offset = *parent_page_idx * PAGE_SIZE + offset;
            return parent_vmo.read_bytes(parent_read_offset, buf);
        }
        self.ensure_page_exists(page_idx)?;
        if self.is_mapped() {
            let page_map_addr = page_idx * PAGE_SIZE + self.mapped_to_addr();
            let vm_space = self.vm_space();
            vm_space.read_bytes(page_map_addr, buf)?;
        } else {
            let inner = self.inner.lock();
            let page_frame = inner.unmapped_pages.get(&page_idx).unwrap();
            page_frame.read_bytes(offset, buf)?;
        }

        Ok(())
    }

    /// commit (and map) page if page not exist
    fn ensure_page_exists(&self, page_idx: usize) -> Result<()> {
        self.commit_page(page_idx * PAGE_SIZE)?;
        let is_mapped = self.is_mapped();
        let inner = self.inner.lock();
        if is_mapped {
            debug_assert!(inner.mapped_pages.contains(&page_idx));
        } else {
            debug_assert!(inner.unmapped_pages.contains_key(&page_idx));
        }
        Ok(())
    }

    pub fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let write_len = buf.len();
        debug_assert!(offset + write_len <= self.size());
        if offset + write_len > self.size() {
            return Err(Error::InvalidArgs);
        }

        let first_page_idx = offset / PAGE_SIZE;
        let last_page_idx = (offset + write_len - 1) / PAGE_SIZE;
        let mut buf_write_offset = 0;
        for page_idx in first_page_idx..=last_page_idx {
            let page_offset = if page_idx == first_page_idx {
                offset - first_page_idx * PAGE_SIZE
            } else {
                0
            };
            let page_remain_len = PAGE_SIZE - page_offset;
            let buf_remain_len = write_len - buf_write_offset;
            let write_len_in_page = page_remain_len.min(buf_remain_len);
            if write_len_in_page == 0 {
                break;
            }
            let write_buf = &buf[buf_write_offset..(buf_write_offset + write_len_in_page)];
            buf_write_offset += write_len_in_page;
            self.write_bytes_in_page(page_idx, page_offset, write_buf)?;
        }
        Ok(())
    }

    fn write_bytes_in_page(&self, page_idx: usize, offset: usize, buf: &[u8]) -> Result<()> {
        // First check if pages in parent
        if let Some(parent_page_idx) = self.inner.lock().inherited_pages.get(&page_idx) {
            match self.vmo_type {
                VmoType::NotChild | VmoType::SliceChild => {
                    let parent_vmo = self.parent.upgrade().unwrap();
                    let parent_read_offset = *parent_page_idx * PAGE_SIZE + offset;
                    return parent_vmo.write_bytes(parent_read_offset, buf);
                }
                VmoType::CopyOnWriteChild => {
                    // Commit a new page for write
                    self.commit_page(page_idx * offset)?;
                    let is_mapped = self.is_mapped();
                    let inner = self.inner.lock();
                    // Copy the content of parent page
                    let mut buffer = [0u8; PAGE_SIZE];
                    let parent_page_idx = inner.inherited_pages.get(&page_idx).unwrap().clone();
                    self.parent
                        .upgrade()
                        .unwrap()
                        .read_bytes(parent_page_idx * PAGE_SIZE, &mut buffer)?;
                    if is_mapped {
                        let mapped_to_addr = inner.mapped_to_addr + page_idx * PAGE_SIZE;
                        let vm_space = inner.mapped_to_vmar.upgrade().unwrap();
                        vm_space.write_bytes(mapped_to_addr, &buffer)?;
                    } else {
                        let frame = inner.unmapped_pages.get(&page_idx).unwrap();
                        frame.write_bytes(0, &buffer)?;
                    }
                }
            }
        }
        self.ensure_page_exists(page_idx)?;
        if self.is_mapped() {
            let page_map_addr = page_idx * PAGE_SIZE + self.mapped_to_addr();
            let vm_space = self.vm_space();
            vm_space.write_bytes(page_map_addr, buf)?;
        } else {
            let inner = self.inner.lock();
            let page_frame = inner.unmapped_pages.get(&page_idx).unwrap();
            page_frame.write_bytes(offset, buf)?;
        }
        Ok(())
    }

    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        todo!()
    }

    pub fn size(&self) -> usize {
        self.inner.lock().size
    }

    pub fn resize(&self, new_size: usize) -> Result<()> {
        todo!()
    }

    pub fn paddr(&self) -> Option<Paddr> {
        self.paddr
    }

    pub fn flags(&self) -> VmoFlags {
        self.flags.clone()
    }

    fn is_mapped(&self) -> bool {
        if self.inner.lock().mapped_to_vmar.strong_count() == 0 {
            true
        } else {
            false
        }
    }

    /// The mapped to vmspace. This function can only be called after self is mapped.
    fn vm_space(&self) -> Arc<VmSpace> {
        let mapped_to_vmar = self.inner.lock().mapped_to_vmar.upgrade().unwrap();
        mapped_to_vmar.vm_space()
    }

    fn mapped_to_addr(&self) -> Vaddr {
        self.inner.lock().mapped_to_addr
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
