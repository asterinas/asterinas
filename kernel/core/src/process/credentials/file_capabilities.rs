// SPDX-License-Identifier: MPL-2.0

use super::{Uid, capabilities::CapSet};
use crate::{
    fs::vfs::{
        inode::Inode,
        xattr::{self, XattrName},
    },
    prelude::*,
};

/// Identifies the `security.capability` xattr revision.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(crate) enum VfsCapRevision {
    V1 = 0x0100_0000,
    V2 = 0x0200_0000,
    V3 = 0x0300_0000,
}

impl VfsCapRevision {
    const MASK: u32 = 0xff00_0000;

    /// Returns the xattr size for this revision.
    pub(crate) const fn xattr_size(self) -> usize {
        match self {
            Self::V1 => 3 * size_of::<u32>(),
            Self::V2 => 5 * size_of::<u32>(),
            Self::V3 => 6 * size_of::<u32>(),
        }
    }
}

bitflags! {
    pub(crate) struct VfsCapFlags: u32 {
        const EFFECTIVE = 0x0000_0001;
    }
}

impl VfsCapFlags {
    const MASK: u32 = 0x00ff_ffff;
}

/// File capabilities loaded from the `security.capability` xattr.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FileCapabilities {
    permitted: CapSet,
    inheritable: CapSet,
    has_effective_flag: bool,
    /// For a V3 xattr, `root_uid` is UID 0 of the writing process's user namespace,
    /// mapped into the initial user namespace.
    /// V1 and V2 xattrs do not store this UID.
    root_uid: Option<Uid>,
}

impl FileCapabilities {
    /// The maximum serialized length of a file capability xattr.
    pub(crate) const MAX_XATTR_SIZE: usize = VfsCapRevision::V3.xattr_size();

    /// Reads file capabilities from an inode's `security.capability` xattr.
    pub(in crate::process) fn read_from_inode(inode: &Arc<dyn Inode>) -> Result<Option<Self>> {
        let xattr_name =
            XattrName::try_from_full_name(xattr::SECURITY_CAPABILITY_XATTR_NAME).unwrap();
        let mut raw_value = [0u8; Self::MAX_XATTR_SIZE];
        let mut value_writer = VmWriter::from(raw_value.as_mut_slice()).to_fallible();
        let value_len = match inode.get_xattr(xattr_name, &mut value_writer) {
            Ok(value_len) => value_len,
            Err(error) if matches!(error.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };

        Self::parse(&raw_value[..value_len]).map(Some)
    }

    pub(in crate::process) const fn permitted(self) -> CapSet {
        self.permitted
    }

    pub(in crate::process) const fn inheritable(self) -> CapSet {
        self.inheritable
    }

    pub(in crate::process) const fn has_effective_flag(self) -> bool {
        self.has_effective_flag
    }

    pub(in crate::process) const fn root_uid(&self) -> Option<Uid> {
        self.root_uid
    }

    /// Parses and validates a `security.capability` xattr header.
    pub(crate) fn parse_header(raw_value: &[u8]) -> Result<(VfsCapRevision, VfsCapFlags)> {
        let header = Self::read_u32_le(raw_value, 0)?;
        let revision_bits = header & VfsCapRevision::MASK;
        let revision = VfsCapRevision::try_from(revision_bits)?;

        let flags_bits = header & VfsCapFlags::MASK;
        let flags = VfsCapFlags::from_bits(flags_bits).ok_or_else(|| {
            Error::with_message(
                Errno::EINVAL,
                "file capabilities contain unsupported flag bits",
            )
        })?;

        if raw_value.len() != revision.xattr_size() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the length of file capabilities does not match its revision"
            );
        }

        Ok((revision, flags))
    }

    fn parse(raw_value: &[u8]) -> Result<Self> {
        let (revision, flags) = Self::parse_header(raw_value)?;

        let (permitted, inheritable) = match revision {
            VfsCapRevision::V1 => (
                CapSet::from_lo_hi(Self::read_u32_le(raw_value, 1)?, 0),
                CapSet::from_lo_hi(Self::read_u32_le(raw_value, 2)?, 0),
            ),
            VfsCapRevision::V2 | VfsCapRevision::V3 => (
                CapSet::from_lo_hi(
                    Self::read_u32_le(raw_value, 1)?,
                    Self::read_u32_le(raw_value, 3)?,
                ),
                CapSet::from_lo_hi(
                    Self::read_u32_le(raw_value, 2)?,
                    Self::read_u32_le(raw_value, 4)?,
                ),
            ),
        };
        let root_uid = match revision {
            VfsCapRevision::V3 => Some(Self::read_v3_uid(raw_value)?),
            VfsCapRevision::V1 | VfsCapRevision::V2 => None,
        };

        Ok(Self {
            permitted,
            inheritable,
            has_effective_flag: flags.contains(VfsCapFlags::EFFECTIVE),
            root_uid,
        })
    }

    /// Reads the root UID from a serialized V3 file-capability xattr.
    pub(crate) fn read_v3_uid(raw_value: &[u8]) -> Result<Uid> {
        Ok(Uid::new(Self::read_u32_le(raw_value, 5)?))
    }

    /// Reads a little-endian word from a serialized file-capability xattr.
    fn read_u32_le(bytes: &[u8], word_index: usize) -> Result<u32> {
        let start = word_index * size_of::<u32>();
        let end = start + size_of::<u32>();
        let Some(word_bytes) = bytes.get(start..end) else {
            return_errno_with_message!(Errno::EINVAL, "file capabilities are truncated");
        };

        let mut word = [0u8; size_of::<u32>()];
        word.copy_from_slice(word_bytes);
        Ok(u32::from_le_bytes(word))
    }
}
