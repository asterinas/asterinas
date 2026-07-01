// SPDX-License-Identifier: MPL-2.0

use super::{Uid, capabilities::CapSet};
use crate::{
    fs::vfs::{
        inode::Inode,
        xattr::{self, XattrName},
    },
    prelude::*,
};

const VFS_CAP_REVISION_MASK: u32 = 0xff00_0000;
const VFS_CAP_FLAGS_MASK: u32 = 0x00ff_ffff;

/// Identifies the `security.capability` xattr revision.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum VfsCapRevision {
    V1 = 0x0100_0000,
    V2 = 0x0200_0000,
    V3 = 0x0300_0000,
}

impl VfsCapRevision {
    /// Returns the xattr size for this revision.
    pub const fn xattr_size(self) -> usize {
        match self {
            Self::V1 => 3 * size_of::<u32>(),
            Self::V2 => 5 * size_of::<u32>(),
            Self::V3 => 6 * size_of::<u32>(),
        }
    }
}

bitflags! {
    struct VfsCapFlags: u32 {
        const EFFECTIVE = 0x0000_0001;
    }
}

const MAX_XATTR_CAPS_SIZE: usize = VfsCapRevision::V3.xattr_size();

/// File capabilities loaded from the `security.capability` xattr.
#[derive(Clone, Copy, Debug)]
pub struct FileCapabilities {
    permitted: CapSet,
    inheritable: CapSet,
    has_effective_flag: bool,
    // V1 and V2 xattrs do not store a root ID. Linux treats those xattrs as
    // bound to UID 0 in the initial user namespace.
    user_ns_owner_uid_in_init_user_ns: Option<Uid>,
}

impl FileCapabilities {
    /// Reads file capabilities from an inode's `security.capability` xattr.
    pub(in crate::process) fn read_from_inode(inode: &Arc<dyn Inode>) -> Result<Option<Self>> {
        let mut raw_value = [0u8; MAX_XATTR_CAPS_SIZE];
        let mut value_writer = VmWriter::from(raw_value.as_mut_slice()).to_fallible();
        let xattr_name = XattrName::try_from_full_name(xattr::SECURITY_CAPABILITY_XATTR_NAME)
            .ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "invalid file capability xattr name")
            })?;
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
        self.user_ns_owner_uid_in_init_user_ns
    }

    /// Parses and validates a `security.capability` xattr header.
    pub fn parse_header(header: u32, value_len: usize) -> Result<VfsCapRevision> {
        let revision_bits = header & VFS_CAP_REVISION_MASK;
        let Ok(revision) = VfsCapRevision::try_from(revision_bits) else {
            return_errno_with_message!(
                Errno::EINVAL,
                "file capabilities use an unsupported xattr revision"
            );
        };

        let flags_bits = header & VFS_CAP_FLAGS_MASK;
        if VfsCapFlags::from_bits(flags_bits).is_none() {
            return_errno_with_message!(
                Errno::EINVAL,
                "file capabilities contain unsupported flag bits"
            );
        }

        if value_len != revision.xattr_size() {
            return_errno_with_message!(
                Errno::EINVAL,
                "file capability xattr length does not match its revision"
            );
        }

        Ok(revision)
    }

    fn parse(raw_value: &[u8]) -> Result<Self> {
        let magic_etc = read_u32_le(raw_value, 0)?;
        let revision = Self::parse_header(magic_etc, raw_value.len())?;
        let flags = VfsCapFlags::from_bits_truncate(magic_etc & VFS_CAP_FLAGS_MASK);

        let (permitted, inheritable) = match revision {
            VfsCapRevision::V1 => (
                CapSet::from_lo_hi(read_u32_le(raw_value, 1)?, 0),
                CapSet::from_lo_hi(read_u32_le(raw_value, 2)?, 0),
            ),
            VfsCapRevision::V2 | VfsCapRevision::V3 => (
                CapSet::from_lo_hi(read_u32_le(raw_value, 1)?, read_u32_le(raw_value, 3)?),
                CapSet::from_lo_hi(read_u32_le(raw_value, 2)?, read_u32_le(raw_value, 4)?),
            ),
        };
        let user_ns_owner_uid_in_init_user_ns = match revision {
            VfsCapRevision::V3 => Some(Uid::new(read_u32_le(raw_value, 5)?)),
            VfsCapRevision::V1 | VfsCapRevision::V2 => None,
        };

        Ok(Self {
            permitted,
            inheritable,
            has_effective_flag: flags.contains(VfsCapFlags::EFFECTIVE),
            user_ns_owner_uid_in_init_user_ns,
        })
    }
}

fn read_u32_le(bytes: &[u8], word_index: usize) -> Result<u32> {
    let start = word_index * size_of::<u32>();
    let end = start + size_of::<u32>();
    let Some(word_bytes) = bytes.get(start..end) else {
        return_errno_with_message!(Errno::EINVAL, "file capability xattr is truncated");
    };

    let mut word = [0u8; size_of::<u32>()];
    word.copy_from_slice(word_bytes);
    Ok(u32::from_le_bytes(word))
}
