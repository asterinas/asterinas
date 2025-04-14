// SPDX-License-Identifier: MPL-2.0

use core::{num::NonZeroUsize, ops::Range};

use ostd::mm::{vm_space::Status, UFrame};

use crate::{
    fs::utils::Inode,
    prelude::*,
    vm::{
        perms::VmPerms,
        vmo::{CommitFlags, Vmo, VmoCommitError},
    },
};

/// A marker directly recorded in the PT rather than the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VmMarker {
    pub(super) perms: VmPerms,
    pub(super) is_shared: bool,
    pub(super) vmo_backed_id: Option<u32>,
}

/// A file-backed VMO mapping.
#[derive(Debug)]
pub(super) struct VmoBackedVMA {
    #[expect(dead_code)]
    pub(super) id: u32,
    /// The size of mapping, in bytes. The map size can even be larger than the
    /// size of VMO. Those pages outside VMO range cannot be read or write.
    ///
    /// Zero sized mapping is not allowed. So this field is always non-zero.
    #[expect(dead_code)]
    pub(super) map_size: NonZeroUsize,
    /// The base address relative to the root VMAR where the VMO is mapped.
    pub(super) map_to_addr: Vaddr,
    /// Specific physical pages that need to be mapped.
    ///
    /// The start of the virtual address maps to the start of the range
    /// specified in [`MappedVmo`].
    pub(super) vmo: MappedVmo,
    /// The inode of the file that backs the mapping.
    ///
    /// If the inode is `Some`, it means that the mapping is file-backed.
    /// And the `vmo` field must be the page cache of the inode.
    pub(super) inode: Option<Arc<dyn Inode>>,
    /// Whether the mapping is shared.
    ///
    /// The updates to a shared mapping are visible among processes, or carried
    /// through to the underlying file for file-backed shared mappings.
    #[expect(dead_code)]
    pub(super) is_shared: bool,
    /// Whether the mapping needs to handle surrounding pages when handling
    /// page fault.
    pub(super) handle_page_faults_around: bool,
}

impl Clone for VmoBackedVMA {
    fn clone(&self) -> Self {
        Self {
            vmo: self.vmo.dup().unwrap(),
            inode: self.inode.as_ref().map(Arc::clone),
            ..*self
        }
    }
}

bitflags! {
    struct VmMarkerToken: usize {
        // Always set
        const MAPPED    = 1 << 0;
        const READ      = 1 << 1;
        const WRITE     = 1 << 2;
        const EXEC      = 1 << 3;
        const SHARED    = 1 << 4;
    }
}

impl VmMarker {
    pub(super) fn encode(self) -> Status {
        let mut token = VmMarkerToken::MAPPED;
        if self.perms.contains(VmPerms::READ) {
            token |= VmMarkerToken::READ;
        }
        if self.perms.contains(VmPerms::WRITE) {
            token |= VmMarkerToken::WRITE;
        }
        if self.perms.contains(VmPerms::EXEC) {
            token |= VmMarkerToken::EXEC;
        }
        if self.is_shared {
            token |= VmMarkerToken::SHARED;
        }
        let mut bits = token.bits();
        if let Some(vmo_backed_id) = self.vmo_backed_id {
            bits |= (vmo_backed_id as usize) << 5;
        }
        Status::try_from(bits).unwrap()
    }

    pub(super) fn decode(token: Status) -> Self {
        let vmo_backed_id = usize::from(token) >> 5;
        let vmo_backed_id = if vmo_backed_id == 0 {
            None
        } else {
            Some(vmo_backed_id as u32)
        };

        let token = VmMarkerToken::from_bits_truncate(token.into());

        debug_assert!(token.contains(VmMarkerToken::MAPPED));

        let mut perms = VmPerms::empty();

        if token.contains(VmMarkerToken::READ) {
            perms |= VmPerms::READ;
        }
        if token.contains(VmMarkerToken::WRITE) {
            perms |= VmPerms::WRITE;
        }
        if token.contains(VmMarkerToken::EXEC) {
            perms |= VmPerms::EXEC;
        }

        let is_shared = token.contains(VmMarkerToken::SHARED);

        Self {
            perms,
            is_shared,
            vmo_backed_id,
        }
    }
}

/// A wrapper that represents a mapped [`Vmo`] and provide required functionalities
/// that need to be provided to mappings from the VMO.
#[derive(Debug)]
pub(super) struct MappedVmo {
    vmo: Vmo,
    /// Represents the mapped offset in the VMO for the mapping.
    offset: usize,
}

impl MappedVmo {
    /// Creates a `MappedVmo` used for the mapping.
    pub(super) fn new(vmo: Vmo, offset: usize) -> Self {
        Self { vmo, offset }
    }

    /// Returns the **valid** size of the `MappedVmo`.
    ///
    /// The **valid** size of a `MappedVmo` is the size of its accessible range
    /// that actually falls within the bounds of the underlying VMO.
    pub(super) fn valid_size(&self) -> usize {
        let vmo_size = self.vmo.size();
        (self.offset..vmo_size).len()
    }

    /// Gets the committed frame at the input offset in the mapped VMO.
    ///
    /// If the VMO has not committed a frame at this index, it will commit
    /// one first and return it.
    pub(super) fn get_committed_frame(
        &self,
        page_offset: usize,
    ) -> core::result::Result<UFrame, VmoCommitError> {
        debug_assert!(page_offset % PAGE_SIZE == 0);
        self.vmo.try_commit_page(self.offset + page_offset)
    }

    /// Commits a page at a specific page index.
    ///
    /// This method may involve I/O operations if the VMO needs to fecth
    /// a page from the underlying page cache.
    pub fn commit_on(&self, page_idx: usize, commit_flags: CommitFlags) -> Result<UFrame> {
        self.vmo.commit_on(page_idx, commit_flags)
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    ///
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    pub(super) fn operate_on_range<F>(
        &self,
        range: &Range<usize>,
        operate: F,
    ) -> core::result::Result<(), VmoCommitError>
    where
        F: FnMut(
            &mut dyn FnMut() -> core::result::Result<UFrame, VmoCommitError>,
        ) -> core::result::Result<(), VmoCommitError>,
    {
        let range = self.offset + range.start..self.offset + range.end;
        self.vmo.try_operate_on_range(&range, operate)
    }

    /// Duplicates the capability.
    pub(super) fn dup(&self) -> Result<Self> {
        Ok(Self {
            vmo: self.vmo.dup()?,
            offset: self.offset,
        })
    }
}
