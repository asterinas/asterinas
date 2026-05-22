// SPDX-License-Identifier: MPL-2.0

use super::{Uid, capabilities::CapSet};
use crate::{
    fs::vfs::{inode::Inode, xattr::XattrName},
    prelude::*,
};

const SECURITY_CAPABILITY_XATTR: &str = "security.capability";

const VFS_CAP_REVISION_MASK: u32 = 0xff00_0000;
const VFS_CAP_FLAGS_MASK: u32 = 0x00ff_ffff;
const VFS_CAP_FLAGS_EFFECTIVE: u32 = 0x0000_0001;

const VFS_CAP_REVISION_1: u32 = 0x0100_0000;
const VFS_CAP_REVISION_2: u32 = 0x0200_0000;
const VFS_CAP_REVISION_3: u32 = 0x0300_0000;

const XATTR_CAPS_SZ_1: usize = 3 * size_of::<u32>();
const XATTR_CAPS_SZ_2: usize = 5 * size_of::<u32>();
const XATTR_CAPS_SZ_3: usize = 6 * size_of::<u32>();

/// File capabilities loaded from the `security.capability` xattr.
#[derive(Clone, Copy, Debug)]
pub(in crate::process) struct FileCapabilities {
    permitted: CapSet,
    inheritable: CapSet,
    has_effective_flag: bool,
    root_uid: Option<Uid>,
}

impl FileCapabilities {
    /// Reads file capabilities from an inode's `security.capability` xattr.
    pub(in crate::process) fn read_from_inode(inode: &Arc<dyn Inode>) -> Result<Option<Self>> {
        let Some(xattr_name) = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR) else {
            unreachable!("`security.capability` should always parse as a valid xattr name");
        };

        let mut raw_value = [0u8; XATTR_CAPS_SZ_3];
        let mut value_writer = VmWriter::from(raw_value.as_mut_slice()).to_fallible();
        let value_len =
            match inode.get_xattr_without_permission_check(xattr_name, &mut value_writer) {
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

    pub(in crate::process) fn applies_to_root_uid(self, root_uid: Uid) -> bool {
        self.root_uid
            .is_none_or(|stored_root_uid| stored_root_uid == root_uid)
    }

    fn parse(raw_value: &[u8]) -> Result<Self> {
        let magic_etc = read_u32_le(raw_value, 0)?;
        let revision = magic_etc & VFS_CAP_REVISION_MASK;
        let flags = magic_etc & VFS_CAP_FLAGS_MASK;
        if flags & !VFS_CAP_FLAGS_EFFECTIVE != 0 {
            return Err(invalid_xattr_error(
                "file capabilities contain unsupported flag bits",
            ));
        }

        let (expected_len, permitted, inheritable, root_uid) = match revision {
            VFS_CAP_REVISION_1 => (
                XATTR_CAPS_SZ_1,
                build_capset(read_u32_le(raw_value, 1)?, 0)?,
                build_capset(read_u32_le(raw_value, 2)?, 0)?,
                None,
            ),
            VFS_CAP_REVISION_2 => (
                XATTR_CAPS_SZ_2,
                build_capset(read_u32_le(raw_value, 1)?, read_u32_le(raw_value, 3)?)?,
                build_capset(read_u32_le(raw_value, 2)?, read_u32_le(raw_value, 4)?)?,
                None,
            ),
            VFS_CAP_REVISION_3 => (
                XATTR_CAPS_SZ_3,
                build_capset(read_u32_le(raw_value, 1)?, read_u32_le(raw_value, 3)?)?,
                build_capset(read_u32_le(raw_value, 2)?, read_u32_le(raw_value, 4)?)?,
                Some(Uid::new(read_u32_le(raw_value, 5)?)),
            ),
            _ => {
                return Err(invalid_xattr_error(
                    "file capabilities use an unsupported xattr revision",
                ));
            }
        };

        if raw_value.len() != expected_len {
            return Err(invalid_xattr_error(
                "file capability xattr length does not match its revision",
            ));
        }

        Ok(Self {
            permitted,
            inheritable,
            has_effective_flag: flags & VFS_CAP_FLAGS_EFFECTIVE != 0,
            root_uid,
        })
    }
}

fn read_u32_le(bytes: &[u8], word_index: usize) -> Result<u32> {
    let start = word_index
        .checked_mul(size_of::<u32>())
        .ok_or_else(|| invalid_xattr_error("file capability xattr index overflowed"))?;
    let end = start + size_of::<u32>();
    let Some(word_bytes) = bytes.get(start..end) else {
        return Err(invalid_xattr_error("file capability xattr is truncated"));
    };

    let mut word = [0u8; size_of::<u32>()];
    word.copy_from_slice(word_bytes);
    Ok(u32::from_le_bytes(word))
}

fn build_capset(lo: u32, hi: u32) -> Result<CapSet> {
    let bits = lo as u64 | ((hi as u64) << 32);
    CapSet::try_from(bits)
        .map_err(|_| invalid_xattr_error("file capabilities contain unsupported capability bits"))
}

fn invalid_xattr_error(message: &'static str) -> Error {
    Error::with_message(Errno::EINVAL, message)
}
