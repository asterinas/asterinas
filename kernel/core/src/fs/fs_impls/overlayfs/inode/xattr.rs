// SPDX-License-Identifier: MPL-2.0

//! Private xattr manipulation
//!
//! Overlayfs keeps its bookkeeping in xattrs that sit beside the user's own: [`OverlayXattrType`]
//! names each private record, and [`OverlayInode`] carries the xattr entries the VFS calls.
//!
//! Every non-private name passes through to the real object unchanged; a name inside the private
//! namespace gains one escape infix, and this mount's own records stay out of presented lists.
//!
//! Copy-up writes the source's xattrs onto the promoted object verbatim, under the same filter.
//!
//! Decoding and meaning live with each record's semantic owner.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/xattrs.c#L7-L34>
//!   (Linux `ovl_is_own_xattr` / `ovl_is_escaped_xattr` / `ovl_is_private_xattr`)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/xattrs.c#L157-L180>
//!   (Linux `ovl_xattr_escape_name` infix insertion and `EOPNOTSUPP` limit)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/xattrs.c#L182-L218>
//!   (Linux `ovl_own_xattr_{get,set}` unconditional escape)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/copy_up.c#L113-L156>
//!   (Linux `ovl_copy_xattr` private-name filter and verbatim copy)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/params.c#L956-L976>
//!   (Linux userxattr redirect/metacopy exclusivity and default disabling)

#![short_vis_path::add(overlayfs)]

use super::{OverlayInode, copyup::workdir::WorkdirTemp};
use crate::{
    fs::vfs::{
        inode::Inode,
        path::Dentry,
        xattr::{XATTR_LIST_MAX_LEN, XATTR_NAME_MAX_LEN, XattrName, XattrNamespace, XattrSetFlags},
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// One overlay-owned record: the name it takes on a real object, and its value as raw bytes.
pub(in overlayfs) enum OverlayXattrType {
    /// The origin record: the lower file that this upper file was copied up from.
    Origin,
    /// The opaque record: the lower entries of this directory are hidden.
    Opaque,
    /// The whiteout record: the lower entry of this name is masked.
    Whiteout,
    /// The impure record: this upper directory may hold origin-adjusted (copy-up) entries.
    Impure,
    /// The uuid record: the identifier of the mount instance that wrote this file.
    Uuid,
}

impl OverlayXattrType {
    /// Builds the xattr name for this record under the mount's selected namespace.
    fn construct_xattr_name(&self, namespace: XattrNamespace) -> Result<XattrName<'static>> {
        let full_name: &'static str = match (namespace, self) {
            (XattrNamespace::Trusted, Self::Origin) => "trusted.overlay.origin",
            (XattrNamespace::Trusted, Self::Opaque) => "trusted.overlay.opaque",
            (XattrNamespace::Trusted, Self::Whiteout) => "trusted.overlay.whiteout",
            (XattrNamespace::Trusted, Self::Impure) => "trusted.overlay.impure",
            (XattrNamespace::Trusted, Self::Uuid) => "trusted.overlay.uuid",
            (XattrNamespace::User, Self::Origin) => "user.overlay.origin",
            (XattrNamespace::User, Self::Opaque) => "user.overlay.opaque",
            (XattrNamespace::User, Self::Whiteout) => "user.overlay.whiteout",
            (XattrNamespace::User, Self::Impure) => "user.overlay.impure",
            (XattrNamespace::User, Self::Uuid) => "user.overlay.uuid",
            _ => unreachable!("the mount stores only the trusted or user xattr namespace"),
        };
        XattrName::try_from_full_name(full_name)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid overlay record xattr name"))
    }

    /// What a write falls back to when the write site supplies none: `y` on the three markers, and
    /// `None` on origin and uuid, whose value the write site supplies.
    fn default_value(&self) -> Option<&'static [u8]> {
        match self {
            Self::Opaque | Self::Whiteout | Self::Impure => Some(b"y"),
            Self::Origin | Self::Uuid => None,
        }
    }

    /// Writes this record on `real_dentry` under `namespace`, replacing any value already stored
    /// there; a `value` of `None` stores the record's own default instead.
    pub(in overlayfs) fn set_value_on(
        &self,
        real_dentry: &Dentry,
        namespace: XattrNamespace,
        value: Option<&[u8]>,
    ) -> Result<()> {
        let value = value.or(self.default_value()).ok_or_else(|| {
            Error::with_message(
                Errno::EINVAL,
                "the overlay xattr type has no value to write",
            )
        })?;
        let name = self.construct_xattr_name(namespace)?;
        let mut value_reader = VmReader::from(value).to_fallible();
        real_dentry.inode().set_xattr(
            real_dentry,
            name,
            &mut value_reader,
            XattrSetFlags::CREATE_OR_REPLACE,
        )
    }

    /// Reads this record's value into `value`; an absent record or a short buffer is an error.
    pub(in overlayfs) fn get_value_from(
        &self,
        real_dentry: &Dentry,
        namespace: XattrNamespace,
        value: &mut [u8],
    ) -> Result<usize> {
        let name = self.construct_xattr_name(namespace)?;
        let mut value_writer = VmWriter::from(value).to_fallible();
        real_dentry.inode().get_xattr(name, &mut value_writer)
    }

    /// Whether `value` is positive for this type: only the two value-semantics markers compare it.
    fn is_value_positive(&self, value: &[u8]) -> bool {
        match self {
            Self::Opaque | Self::Whiteout => value == self.default_value().unwrap_or_default(),
            Self::Impure | Self::Origin | Self::Uuid => true,
        }
    }

    /// Whether this record is present on `real_dentry` with a value this type treats as positive.
    pub(super) fn is_positive_on(
        &self,
        real_dentry: &Dentry,
        namespace: XattrNamespace,
    ) -> Result<bool> {
        let mut value = [0u8; 1];
        let written = match self.get_value_from(real_dentry, namespace, &mut value) {
            Ok(written) => written,
            Err(err) if err.error() == Errno::ERANGE => 0,
            Err(err) if matches!(err.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => {
                return Ok(false);
            }
            Err(err) => return Err(err),
        };
        Ok(self.is_value_positive(&value[..written]))
    }

    /// Removes this record, reporting `ENODATA` if the record is not there.
    pub(super) fn remove_from(
        &self,
        real_dentry: &Dentry,
        namespace: XattrNamespace,
    ) -> Result<()> {
        let name = self.construct_xattr_name(namespace)?;
        real_dentry.inode().remove_xattr(real_dentry, name)
    }
}

impl OverlayInode {
    /// The private-namespace prefix this mount selected.
    fn private_xattr_prefix(&self) -> &'static str {
        const TRUSTED_OVERLAY_PREFIX: &str = "trusted.overlay.";
        const USER_OVERLAY_PREFIX: &str = "user.overlay.";
        if self.fs_arc().policy().xattr_namespace() == XattrNamespace::User {
            USER_OVERLAY_PREFIX
        } else {
            TRUSTED_OVERLAY_PREFIX
        }
    }

    /// Whether `name` lies in the mount's private namespace, which is a prefix test.
    fn is_own_xattr_name(&self, name: &str) -> bool {
        name.starts_with(self.private_xattr_prefix())
    }

    /// Whether `name` already carries an escape infix, the mark of an overlay stacked below.
    fn is_escaped_xattr_name(&self, name: &str) -> bool {
        const ESCAPE_INFIX: &str = "overlay.";
        name.strip_prefix(self.private_xattr_prefix())
            .is_some_and(|suffix| suffix.starts_with(ESCAPE_INFIX))
    }

    /// This mount's own record: inside the private namespace and not already escaped.
    fn is_private_xattr_name(&self, name: &str) -> bool {
        self.is_own_xattr_name(name) && !self.is_escaped_xattr_name(name)
    }

    /// The real name for a VFS-facing name: a name inside this mount's namespace gains one escape
    /// infix after the prefix, keeping a stacked mount's records of one name apart from ours.
    fn xattr_name_for_real(&self, name: &XattrName) -> Result<String> {
        const ESCAPE_INFIX: &str = "overlay.";
        let selected_prefix = self.private_xattr_prefix();
        let full_name = name.full_name();
        if !self.is_own_xattr_name(full_name) {
            return Ok(String::from(full_name));
        }
        if full_name.len() + ESCAPE_INFIX.len() > XATTR_NAME_MAX_LEN {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "the escaped overlay xattr name exceeds the xattr name length limit"
            );
        }
        let mut used_name = String::from(full_name);
        used_name.insert_str(selected_prefix.len(), ESCAPE_INFIX);
        Ok(used_name)
    }

    /// The VFS-facing name for a real name, or `None` for this mount's own record: the escape infix
    /// this mount inserted is removed from the presented name.
    fn xattr_name_for_vfs(&self, name: &str) -> Option<String> {
        const ESCAPE_INFIX: &str = "overlay.";
        let selected_prefix = self.private_xattr_prefix();
        let Some(suffix) = name.strip_prefix(selected_prefix) else {
            return Some(String::from(name));
        };
        let Some(stripped_suffix) = suffix.strip_prefix(ESCAPE_INFIX) else {
            // Inside the namespace with no infix: this mount's own record, hidden from the list.
            return None;
        };
        let mut presented_name = String::from(selected_prefix);
        presented_name.push_str(stripped_suffix);
        Some(presented_name)
    }

    pub(super) fn get_xattr_impl(
        &self,
        name: XattrName,
        value_writer: &mut VmWriter,
    ) -> Result<usize> {
        let used_name = self.xattr_name_for_real(&name)?;
        // The `EINVAL` arm is unreachable: the infix is inserted inside the selected namespace.
        let used = XattrName::try_from_full_name(&used_name).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid escaped overlay xattr name")
        })?;
        self.real_object()
            .real_inode()
            .get_xattr(used, value_writer)
    }

    pub(super) fn set_xattr_impl(
        &self,
        self_dentry: &Dentry,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        let used_name = self.xattr_name_for_real(&name)?;
        let used = XattrName::try_from_full_name(&used_name).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid escaped overlay xattr name")
        })?;
        let upper = self.writable_real_object(self_dentry)?;
        upper
            .real_inode()
            .set_xattr(upper.dentry(), used, value_reader, flags)
    }

    pub(super) fn remove_xattr_impl(&self, self_dentry: &Dentry, name: XattrName) -> Result<()> {
        let used_name = self.xattr_name_for_real(&name)?;
        let used = XattrName::try_from_full_name(&used_name).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid escaped overlay xattr name")
        })?;
        let upper = self.writable_real_object(self_dentry)?;
        upper.real_inode().remove_xattr(upper.dentry(), used)
    }

    /// Presents the real list: this mount's own records are hidden, one escape infix is stripped,
    /// so a name this mount escaped reads as the caller's name while nested infixes survive.
    pub(super) fn list_xattr_impl(
        &self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        let real = self.real_object();
        let mut raw_list = vec![0u8; XATTR_LIST_MAX_LEN];
        let mut raw_writer = VmWriter::from(&mut raw_list[..]).to_fallible();
        let list_len = real.real_inode().list_xattr(namespace, &mut raw_writer)?;
        let mut bytes_written = 0;
        for name_bytes in raw_list[..list_len].split(|&byte| byte == 0) {
            if name_bytes.is_empty() {
                continue;
            }
            // A name that is not UTF-8 is passed through unchanged.
            let presented = match core::str::from_utf8(name_bytes) {
                Ok(name) => match self.xattr_name_for_vfs(name) {
                    Some(presented_name) => presented_name.into_bytes(),
                    None => continue,
                },
                Err(_) => name_bytes.to_vec(),
            };
            let entry_len = presented.len() + 1;
            // A zero-length buffer is the size probe: entries are counted, not written.
            if list_writer.avail() == 0 {
                bytes_written += entry_len;
                continue;
            }
            // An entry that does not fit fails the call; earlier entries stay and none is split.
            if entry_len > list_writer.avail() {
                return_errno_with_message!(
                    Errno::ERANGE,
                    "the xattr list buffer is too small for the presented list"
                );
            }
            list_writer.write_fallible(&mut VmReader::from(presented.as_slice()))?;
            list_writer.write_val(&0u8)?;
            bytes_written += entry_len;
        }
        Ok(bytes_written)
    }

    /// Copies `source`'s non-private xattrs onto `temp`; `tolerate_errors` skips a failing xattr.
    pub(super) fn copy_eligible_xattrs(
        &self,
        source: &Arc<dyn Inode>,
        temp: &WorkdirTemp,
        tolerate_errors: bool,
    ) -> Result<()> {
        for namespace in [
            XattrNamespace::User,
            XattrNamespace::Trusted,
            XattrNamespace::Security,
        ] {
            let names = match self.probe_source_xattr_names(source, namespace) {
                Ok(names) => names,
                Err(err)
                    if tolerate_errors || matches!(err.error(), Errno::ENODATA | Errno::ERANGE) =>
                {
                    warn!(
                        "overlay xattr copy: source xattr list unavailable for {:?}; \
                         skipping this namespace: {:?}",
                        namespace, err
                    );
                    continue;
                }
                Err(err) => return Err(err),
            };
            for full_name in names
                .split(|&byte| byte == 0)
                .filter(|name| !name.is_empty())
            {
                let Ok(full_name) = core::str::from_utf8(full_name) else {
                    continue;
                };
                // This mount's own records never cross copy-up; an escaped record of another
                // overlay is not ours, so it is copied under its own name.
                if self.is_private_xattr_name(full_name) {
                    continue;
                }
                let Some(name) = XattrName::try_from_full_name(full_name) else {
                    warn!(
                        "overlay xattr copy: skipping unparsable xattr name: {}",
                        full_name
                    );
                    continue;
                };
                if name.namespace() != namespace {
                    continue;
                }
                let value = match self.probe_source_xattr_value(source, &name) {
                    Ok(value) => value,
                    Err(err)
                        if tolerate_errors
                            || matches!(err.error(), Errno::ENODATA | Errno::ERANGE) =>
                    {
                        warn!("overlay xattr copy: skipping {}: {:?}", full_name, err);
                        continue;
                    }
                    Err(err) => return Err(err),
                };
                let mut reader = VmReader::from(value.as_slice()).to_fallible();
                match temp.inode().set_xattr(
                    temp.dentry(),
                    name,
                    &mut reader,
                    XattrSetFlags::CREATE_OR_REPLACE,
                ) {
                    Err(err) if tolerate_errors => {
                        warn!(
                            "overlay xattr copy: skipping {} on temp: {:?}",
                            full_name, err
                        );
                        continue;
                    }
                    result => result?,
                }
            }
        }
        Ok(())
    }

    /// The source's raw xattr name list for one namespace, measured in two passes.
    fn probe_source_xattr_names(
        &self,
        source: &Arc<dyn Inode>,
        namespace: XattrNamespace,
    ) -> Result<Vec<u8>> {
        let list_len = {
            let mut probe = VmWriter::from(&mut [] as &mut [u8]).to_fallible();
            source.list_xattr(namespace, &mut probe)?
        };
        let mut names = vec![0u8; list_len];
        let written_len = {
            let mut list_writer = VmWriter::from(names.as_mut_slice()).to_fallible();
            source.list_xattr(namespace, &mut list_writer)?
        };
        names.truncate(written_len);
        Ok(names)
    }

    /// One value of the source by name, re-parsed, then measured and filled in two passes.
    fn probe_source_xattr_value(
        &self,
        source: &Arc<dyn Inode>,
        name: &XattrName<'_>,
    ) -> Result<Vec<u8>> {
        let Some(reborrowed) = XattrName::try_from_full_name(name.full_name()) else {
            return Err(Error::with_message(
                Errno::EINVAL,
                "the xattr name is not a valid escaped name",
            ));
        };
        let mut probe = VmWriter::from(&mut [] as &mut [u8]).to_fallible();
        let value_len = source.get_xattr(reborrowed, &mut probe)?;
        let mut value = vec![0u8; value_len];
        let mut value_writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        let written = source.get_xattr(reborrowed, &mut value_writer)?;
        value.truncate(written);
        Ok(value)
    }
}

#[cfg(ktest)]
mod test {
    //! Unit tests for the name each record takes under either namespace, for the marker's default
    //! value, and for the value table of the one-byte presence probe.
    //!
    //! Every case drives one method of [`OverlayXattrType`] and asserts the value that comes back.

    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn construct_xattr_name_uses_selected_namespace() {
        for (record, trusted, user) in [
            (
                OverlayXattrType::Origin,
                "trusted.overlay.origin",
                "user.overlay.origin",
            ),
            (
                OverlayXattrType::Opaque,
                "trusted.overlay.opaque",
                "user.overlay.opaque",
            ),
            (
                OverlayXattrType::Whiteout,
                "trusted.overlay.whiteout",
                "user.overlay.whiteout",
            ),
            (
                OverlayXattrType::Impure,
                "trusted.overlay.impure",
                "user.overlay.impure",
            ),
            (
                OverlayXattrType::Uuid,
                "trusted.overlay.uuid",
                "user.overlay.uuid",
            ),
        ] {
            assert_eq!(
                record
                    .construct_xattr_name(XattrNamespace::Trusted)
                    .unwrap()
                    .full_name(),
                trusted
            );
            assert_eq!(
                record
                    .construct_xattr_name(XattrNamespace::User)
                    .unwrap()
                    .full_name(),
                user
            );
        }
    }

    #[ktest]
    fn default_value_is_the_marker_value_for_markers_only() {
        for marker in [
            OverlayXattrType::Opaque,
            OverlayXattrType::Whiteout,
            OverlayXattrType::Impure,
        ] {
            assert_eq!(marker.default_value(), Some(b"y".as_slice()));
        }
        for owner_supplied in [OverlayXattrType::Origin, OverlayXattrType::Uuid] {
            assert_eq!(owner_supplied.default_value(), None);
        }
    }

    #[ktest]
    fn is_value_positive_pins_the_probe_table() {
        // Value semantics: only the marker's own value, `b"y"`, is positive.
        for marker in [OverlayXattrType::Opaque, OverlayXattrType::Whiteout] {
            assert!(marker.is_value_positive(b"y"));
            assert!(!marker.is_value_positive(b"n"));
            assert!(!marker.is_value_positive(b"yy"));
            // The empty value is what the one-byte probe hands over for a stored
            // value longer than one byte: `ERANGE` collapses to zero bytes.
            assert!(!marker.is_value_positive(b""));
        }
        // Presence semantics: any value is positive, the empty one included.
        assert!(OverlayXattrType::Impure.is_value_positive(b"y"));
        assert!(OverlayXattrType::Impure.is_value_positive(b"n"));
        assert!(OverlayXattrType::Impure.is_value_positive(b""));
    }
}
