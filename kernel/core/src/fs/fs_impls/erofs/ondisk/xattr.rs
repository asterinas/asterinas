// SPDX-License-Identifier: MPL-2.0

//! Complete immutable inline/traditional-shared attributes and checked staging.
//!
//! Local parsing checks every inline entry and shared index before device reads.
//! Staging can finish only after every declared shared reference has a checked
//! entry. The complete collection keeps its inode-local footprint independently
//! of shared storage; ACL and capability values remain uninterpreted bytes.
//!
//! Format: <https://erofs.docs.kernel.org/en/latest/ondisk/xattrs.html>. Short
//! name indexes support user, trusted, security, and POSIX ACL
//! namespaces; long prefixes, Lustre, and unknown indexes are rejected. ACL
//! suffixes must be empty, other suffixes nonempty; names must be valid UTF-8
//! without NUL and fit the VFS name-length limit. Duplicate namespace/suffix
//! pairs across inline and shared entries are rejected.

use alloc::collections::BTreeSet;
use core::str;

use crate::{
    fs::vfs::xattr::XATTR_NAME_MAX_LEN,
    prelude::{Box, Vec},
};

const BODY_HEADER_SIZE_BYTES: usize = 12;
const ENTRY_HEADER_SIZE_BYTES: usize = 4;
const XATTR_ALIGNMENT_BYTES: usize = 4;
const SHARED_COUNT_OFFSET: usize = 4;
const RESERVED_START: usize = 5;
const LONG_PREFIX_FLAG: u8 = 0x80;

/// Returns the inode-local byte size, including its header and shared index array.
pub(in super::super) fn inline_xattr_size(icount: u16) -> usize {
    if icount == 0 {
        return 0;
    }
    // A u16 count bounds the complete region to 262148 bytes.
    (usize::from(icount) - 1) * XATTR_ALIGNMENT_BYTES + BODY_HEADER_SIZE_BYTES
}

/// Returns the complete aligned size declared by one inline or shared header.
pub(in super::super) fn shared_entry_size(header: &[u8; 4]) -> usize {
    // The u8 name and u16 value lengths bound the aligned entry to 65796 bytes.
    let size = ENTRY_HEADER_SIZE_BYTES
        + usize::from(header[0])
        + usize::from(u16::from_le_bytes([header[2], header[3]]));
    size.next_multiple_of(XATTR_ALIGNMENT_BYTES)
}

/// A complete checked set; no unconsumed shared references survive construction.
#[derive(Debug)]
pub(in super::super) struct InodeXattrs {
    local_size_bytes: usize,
    entries: Vec<XattrEntry>,
    has_access_acl: bool,
}

impl InodeXattrs {
    /// Returns the complete checked entries as an immutable slice.
    pub(in super::super) fn entries(&self) -> &[XattrEntry] {
        &self.entries
    }

    /// Finds a checked name without scanning the full attribute collection.
    pub(in super::super) fn find(
        &self,
        namespace: ShortXattrNamespace,
        suffix: &str,
    ) -> Option<&XattrEntry> {
        self.entries
            .binary_search_by(|entry| (entry.namespace, entry.suffix()).cmp(&(namespace, suffix)))
            .ok()
            .map(|index| &self.entries[index])
    }

    /// Returns only the inode-local disk footprint, never shared storage size.
    pub(in super::super) const fn size_bytes(&self) -> usize {
        self.local_size_bytes
    }
    #[expect(dead_code)]
    pub(in super::super) fn entry_count(&self) -> usize {
        self.entries.len()
    }
    #[expect(dead_code)]
    pub(in super::super) fn entry(&self, index: usize) -> Option<&XattrEntry> {
        self.entries.get(index)
    }
    pub(in super::super) const fn requires_access_acl_consumer(&self) -> bool {
        self.has_access_acl
    }
}

/// Checked local metadata with an ordered, still-unconsumed shared index list.
/// This is deliberately not an attribute set accepted by inode descriptions.
#[derive(Debug)]
pub(in super::super) struct LocalXattrs {
    local_size_bytes: usize,
    entries: Vec<XattrEntry>,
    names: BTreeSet<(ShortXattrNamespace, Box<str>)>,
    shared_indices: Vec<u32>,
    loaded_shared: usize,
}

impl LocalXattrs {
    pub(in super::super) fn parse(icount: u16, bytes: Vec<u8>) -> Result<Self, XattrParseError> {
        let length = inline_xattr_size(icount);
        if bytes.len() != length {
            return Err(XattrParseError::Corrupted(XattrCorruption::RegionLength));
        }
        if icount == 1 {
            return Err(XattrParseError::Unsupported(UnsupportedXattr::HeaderOnly));
        }
        let mut local = Self {
            local_size_bytes: length,
            entries: Vec::new(),
            names: BTreeSet::new(),
            shared_indices: Vec::new(),
            loaded_shared: 0,
        };
        if icount == 0 {
            return Ok(local);
        }
        // An exact icount > 1 region includes this entire fixed body header.
        if bytes[RESERVED_START..BODY_HEADER_SIZE_BYTES]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(XattrParseError::Corrupted(XattrCorruption::NonzeroReserved));
        }
        let array_error = XattrParseError::Corrupted(XattrCorruption::SharedArrayRange);
        let array_end = usize::from(bytes[SHARED_COUNT_OFFSET])
            .checked_mul(4)
            .and_then(|size| BODY_HEADER_SIZE_BYTES.checked_add(size))
            .ok_or(array_error)?;
        let indices = bytes
            .get(BODY_HEADER_SIZE_BYTES..array_end)
            .ok_or(array_error)?;
        for raw in indices.as_chunks::<4>().0 {
            local
                .shared_indices
                .push(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]));
        }
        let invalid_entry = XattrParseError::Corrupted(XattrCorruption::EntryRange);
        let mut cursor = array_end;
        while cursor < bytes.len() {
            let header_end = cursor.checked_add(4).ok_or(invalid_entry)?;
            let header = bytes.get(cursor..header_end).ok_or(invalid_entry)?;
            let header: &[u8; 4] = header.try_into().map_err(|_| invalid_entry)?;
            let end = cursor
                .checked_add(shared_entry_size(header))
                .ok_or(invalid_entry)?;
            let entry = checked_entry(bytes.get(cursor..end).ok_or(invalid_entry)?)?;
            local.add_entry(entry)?;
            cursor = end;
        }
        Ok(local)
    }

    pub(in super::super) fn has_shared(&self) -> bool {
        !self.shared_indices.is_empty()
    }

    pub(in super::super) fn next_shared_index(&self) -> Option<u32> {
        self.shared_indices.get(self.loaded_shared).copied()
    }

    /// Accepts exactly the next declared reference, checking its complete bytes.
    /// The loader supplies the bytes from that index in the same stable device.
    pub(in super::super) fn add_shared(
        &mut self,
        index: u32,
        bytes: Vec<u8>,
    ) -> Result<(), XattrParseError> {
        if self.next_shared_index() != Some(index) {
            return Err(XattrParseError::Corrupted(
                XattrCorruption::SharedEntrySequence,
            ));
        }
        self.add_entry(checked_entry(&bytes)?)?;
        // The matched index is within a header's u8-sized shared count.
        self.loaded_shared += 1;
        Ok(())
    }

    pub(in super::super) fn finish(mut self) -> Result<InodeXattrs, XattrParseError> {
        if self.loaded_shared != self.shared_indices.len() {
            return Err(XattrParseError::Corrupted(
                XattrCorruption::SharedEntrySequence,
            ));
        }
        let has_access_acl = self
            .entries
            .iter()
            .any(|entry| entry.namespace == ShortXattrNamespace::AccessAcl);
        // Inodes are immutable. Sort once so repeated VFS lookups need no
        // separate index, key allocation, or linear attribute scan.
        self.entries.sort_unstable_by(|left, right| {
            (left.namespace, left.suffix()).cmp(&(right.namespace, right.suffix()))
        });
        Ok(InodeXattrs {
            local_size_bytes: self.local_size_bytes,
            entries: self.entries,
            has_access_acl,
        })
    }

    fn add_entry(&mut self, entry: XattrEntry) -> Result<(), XattrParseError> {
        if !self.names.insert((entry.namespace, entry.suffix.clone())) {
            return Err(XattrParseError::Unsupported(
                UnsupportedXattr::DuplicateName,
            ));
        }
        self.entries.push(entry);
        Ok(())
    }
}

/// One immutable, checked name and exact binary value.
#[derive(Debug)]
pub(in super::super) struct XattrEntry {
    namespace: ShortXattrNamespace,
    suffix: Box<str>,
    value: Box<[u8]>,
}

impl XattrEntry {
    pub(in super::super) const fn namespace(&self) -> ShortXattrNamespace {
        self.namespace
    }
    pub(in super::super) fn suffix(&self) -> &str {
        &self.suffix
    }
    pub(in super::super) fn value(&self) -> &[u8] {
        &self.value
    }
}

fn checked_entry(bytes: &[u8]) -> Result<XattrEntry, XattrParseError> {
    let invalid = XattrParseError::Corrupted(XattrCorruption::EntryRange);
    let header: &[u8; 4] = bytes
        .get(..4)
        .ok_or(invalid)?
        .try_into()
        .map_err(|_| invalid)?;
    if shared_entry_size(header) != bytes.len() {
        return Err(invalid);
    }
    let suffix_size = usize::from(header[0]);
    let suffix_end = ENTRY_HEADER_SIZE_BYTES
        .checked_add(suffix_size)
        .ok_or(invalid)?;
    let value_end = suffix_end
        .checked_add(usize::from(u16::from_le_bytes([header[2], header[3]])))
        .ok_or(invalid)?;
    let namespace = ShortXattrNamespace::from_index(header[1])?;
    let suffix_bytes = bytes
        .get(ENTRY_HEADER_SIZE_BYTES..suffix_end)
        .ok_or(invalid)?;
    let suffix = str::from_utf8(suffix_bytes)
        .map_err(|_| XattrParseError::Unsupported(UnsupportedXattr::NameEncoding))?;
    if suffix_bytes.contains(&0) {
        return Err(XattrParseError::Unsupported(UnsupportedXattr::NameEncoding));
    }
    let is_acl = matches!(
        namespace,
        ShortXattrNamespace::AccessAcl | ShortXattrNamespace::DefaultAcl
    );
    if is_acl != suffix.is_empty() {
        return Err(XattrParseError::Unsupported(UnsupportedXattr::NameShape));
    }
    let name_size = namespace
        .prefix()
        .len()
        .checked_add(suffix_size)
        .ok_or(XattrParseError::Unsupported(UnsupportedXattr::NameLength))?;
    if name_size > XATTR_NAME_MAX_LEN {
        return Err(XattrParseError::Unsupported(UnsupportedXattr::NameLength));
    }
    // Entry padding is aligned and present, but it has no must-zero contract.
    Ok(XattrEntry {
        namespace,
        suffix: Box::from(suffix),
        value: Box::from(bytes.get(suffix_end..value_end).ok_or(invalid)?),
    })
}

/// Supported short disk name indexes; ACL variants select complete names.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in super::super) enum ShortXattrNamespace {
    User,
    AccessAcl,
    DefaultAcl,
    Trusted,
    Security,
}

impl ShortXattrNamespace {
    fn from_index(index: u8) -> Result<Self, XattrParseError> {
        let namespace = match index {
            _ if index & LONG_PREFIX_FLAG != 0 => {
                return Err(XattrParseError::Unsupported(UnsupportedXattr::LongPrefix));
            }
            1 => Self::User,
            2 => Self::AccessAcl,
            3 => Self::DefaultAcl,
            4 => Self::Trusted,
            5 => return Err(XattrParseError::Unsupported(UnsupportedXattr::Lustre)),
            6 => Self::Security,
            other => {
                return Err(XattrParseError::Unsupported(UnsupportedXattr::Namespace(
                    other,
                )));
            }
        };
        Ok(namespace)
    }

    /// Returns the standard prefix, or the complete ACL attribute name.
    pub(in super::super) const fn prefix(self) -> &'static str {
        match self {
            Self::User => "user.",
            Self::AccessAcl => "system.posix_acl_access",
            Self::DefaultAcl => "system.posix_acl_default",
            Self::Trusted => "trusted.",
            Self::Security => "security.",
        }
    }
}

/// A structural violation or a separate project support restriction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum XattrParseError {
    Corrupted(XattrCorruption),
    Unsupported(UnsupportedXattr),
}

/// An invalid disk range or an incomplete staged-construction sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum XattrCorruption {
    RegionLength,
    NonzeroReserved,
    SharedArrayRange,
    EntryRange,
    SharedEntrySequence,
}

/// A representation, address space, or naming policy outside this entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum UnsupportedXattr {
    HeaderOnly,
    Metabox,
    LongPrefix,
    Lustre,
    Namespace(u8),
    NameEncoding,
    NameShape,
    NameLength,
    DuplicateName,
}
