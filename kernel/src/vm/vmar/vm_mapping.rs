// SPDX-License-Identifier: MPL-2.0

use core::{num::NonZeroUsize, ops::Range};

use ostd::mm::{vm_space::Token, UFrame};

use crate::{
    prelude::*,
    vm::{perms::VmPerms, vmo::Vmo},
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
    #[allow(dead_code)]
    pub(super) id: u32,
    /// The size of mapping, in bytes. The map size can even be larger than the
    /// size of VMO. Those pages outside VMO range cannot be read or write.
    ///
    /// Zero sized mapping is not allowed. So this field is always non-zero.
    #[allow(dead_code)]
    pub(super) map_size: NonZeroUsize,
    /// The base address relative to the root VMAR where the VMO is mapped.
    pub(super) map_to_addr: Vaddr,
    /// Specific physical pages that need to be mapped.
    ///
    /// The start of the virtual address maps to the start of the range
    /// specified in [`MappedVmo`].
    pub(super) vmo: MappedVmo,
    /// Whether the mapping needs to handle surrounding pages when handling
    /// page fault.
    #[allow(dead_code)]
    pub(super) handle_page_faults_around: bool,
}

impl Clone for VmoBackedVMA {
    fn clone(&self) -> Self {
        Self {
            vmo: self.vmo.dup().unwrap(),
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
    pub(super) fn encode(self) -> Token {
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
        Token::try_from(bits).unwrap()
    }

    pub(super) fn decode(token: Token) -> Self {
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
    /// Represents the accessible range in the VMO for mappings.
    range: Range<usize>,
}

impl MappedVmo {
    /// Creates a `MappedVmo` used for mapping.
    pub(super) fn new(vmo: Vmo, range: Range<usize>) -> Self {
        Self { vmo, range }
    }

    /// Gets the committed frame at the input offset in the mapped VMO.
    ///
    /// If the VMO has not committed a frame at this index, it will commit
    /// one first and return it.
    pub(super) fn get_committed_frame(&self, page_offset: usize) -> Result<UFrame> {
        debug_assert!(page_offset < self.range.len());
        debug_assert!(page_offset % PAGE_SIZE == 0);
        self.vmo.commit_page(self.range.start + page_offset)
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    ///
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    #[allow(dead_code)]
    pub(super) fn operate_on_range<F>(&self, range: &Range<usize>, operate: F) -> Result<()>
    where
        F: FnMut(&mut dyn FnMut() -> Result<UFrame>) -> Result<()>,
    {
        debug_assert!(range.start < self.range.len());
        debug_assert!(range.end <= self.range.len());

        let range = self.range.start + range.start..self.range.start + range.end;

        self.vmo.operate_on_range(&range, operate)
    }

    /// Duplicates the capability.
    pub(super) fn dup(&self) -> Result<Self> {
        Ok(Self {
            vmo: self.vmo.dup()?,
            range: self.range.clone(),
        })
    }
}
