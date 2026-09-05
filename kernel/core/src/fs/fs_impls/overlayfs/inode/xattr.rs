// SPDX-License-Identifier: MPL-2.0
#![short_vis_path::add(overlayfs)]

//! The xattr two-path policy: the private record path and the passthrough
//! path.
//!
//! Every xattr full name is classified into one of two classes by prefix
//! only:
//!
//! - `Private`: the name starts with the mount's selected private prefix —
//!   `trusted.overlay.` by default, `user.overlay.` in `userxattr` mode.
//!   These are the overlay's own records (origin, opaque, whiteout, impure,
//!   uuid); they never cross copy-up and are hidden from the visible list.
//! - `Passthrough`: every other name (`user.plain.any`, `security.selinux`,
//!   `trusted.backup.notes`, ...); passed through unchanged, never
//!   interpreted.
//!
//! # The two paths
//!
//! - **Private path** ([`OverlayInode::set_overlay_xattr`] plus the raw
//!   record reads): the name reaches the real object unchanged — the escape
//!   infix is never inserted. Every internal overlay write routes through
//!   this entry, so the un-escaped-name invariant has one enforcement point.
//! - **Passthrough path** (the `*_impl` entries): an own-prefix name is
//!   shifted one segment deeper ([`ESCAPE_INFIX`] inserted right after the
//!   selected prefix) before it reaches the real authority — unconditionally,
//!   even for a name that already carries the infix. The list transform
//!   ([`present_xattr_names`]) is the inverse map: own private records are
//!   hidden and one infix segment is stripped per layer. The mutating
//!   entries (`set_xattr`, `remove_xattr`) admit through
//!   [`OverlayInode::check_mutating_permission`] before delegating.
//!
//! Stacked same-prefix overlays therefore physically layer their records by
//! infix-segment count: the count equals the number of overlays between the
//! record's owner and the backing filesystem, and each layer's passthrough
//! path adds exactly one segment while each layer's list transform strips
//! exactly one.
//!
//! # userxattr feature exclusions (enforced at mount time)
//!
//! `userxattr` excludes the `redirect_dir`/`metacopy` features. The option
//! verify phase rejects the explicit combinations — `userxattr` +
//! `redirect_dir`≠`nofollow` → `EINVAL`; `userxattr` + `metacopy=on` →
//! `EINVAL` — and both features remain unimplemented, so every other
//! explicit request degrades with a disclosed one-shot warning. The
//! exclusivity contract is recorded here because the private-prefix
//! decision and the option surface must stay consistent.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/xattrs.c#L157-L180>
//!   (Linux `ovl_xattr_escape_name` infix insertion and `EOPNOTSUPP` limit)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/xattrs.c#L182-L218>
//!   (Linux `ovl_own_xattr_{get,set}` unconditional escape)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/params.c#L956-L976>
//!   (Linux userxattr redirect/metacopy exclusivity and default disabling)

use super::{
    OverlayInode, ReaddirIndex,
    copyup::workdir::WorkdirTemp,
    permission::{AccessType, CopyUpOrigin},
};
use crate::{
    fs::{
        file::Permission,
        fs_impls::overlayfs::fs::OverlayFs,
        vfs::{
            inode::Inode,
            path::{Dentry, Path},
            xattr::{
                XATTR_LIST_MAX_LEN, XATTR_NAME_MAX_LEN, XattrName, XattrNamespace, XattrSetFlags,
            },
        },
    },
    prelude::*,
};

/// Writing `trusted.*` requires `CAP_SYS_ADMIN`, shielding the overlay's
/// own records from non-root users.
const TRUSTED_OVERLAY_PREFIX: &str = "trusted.overlay.";

/// `user.overlay.` is world-writable, so it cannot shield the overlay's own
/// records the way `trusted.*` does.
const USER_OVERLAY_PREFIX: &str = "user.overlay.";

const ESCAPE_INFIX: &str = "overlay.";

const TRUSTED_OVERLAY_ORIGIN: &str = "trusted.overlay.origin";
const USER_OVERLAY_ORIGIN: &str = "user.overlay.origin";
const TRUSTED_OVERLAY_OPAQUE: &str = "trusted.overlay.opaque";
const USER_OVERLAY_OPAQUE: &str = "user.overlay.opaque";
const TRUSTED_OVERLAY_WHITEOUT: &str = "trusted.overlay.whiteout";
const USER_OVERLAY_WHITEOUT: &str = "user.overlay.whiteout";
const TRUSTED_OVERLAY_IMPURE: &str = "trusted.overlay.impure";
const USER_OVERLAY_IMPURE: &str = "user.overlay.impure";
const TRUSTED_OVERLAY_UUID: &str = "trusted.overlay.uuid";
const USER_OVERLAY_UUID: &str = "user.overlay.uuid";

const OPAQUE_MARKER_VALUE: &[u8] = b"y";

pub(super) const WHITEOUT_MARKER_VALUE: &[u8] = b"y";

const IMPURE_MARKER_VALUE: &[u8] = b"y";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) enum OverlayRecordName {
    Origin,
    Opaque,
    Whiteout,
    Impure,
    Uuid,
}

pub(in overlayfs) fn overlay_record_name(
    record: OverlayRecordName,
    prefix: OverlayXattrPrefix,
) -> Result<XattrName<'static>> {
    let full_name: &'static str = match (record, prefix) {
        (OverlayRecordName::Origin, OverlayXattrPrefix::Trusted) => TRUSTED_OVERLAY_ORIGIN,
        (OverlayRecordName::Origin, OverlayXattrPrefix::User) => USER_OVERLAY_ORIGIN,
        (OverlayRecordName::Opaque, OverlayXattrPrefix::Trusted) => TRUSTED_OVERLAY_OPAQUE,
        (OverlayRecordName::Opaque, OverlayXattrPrefix::User) => USER_OVERLAY_OPAQUE,
        (OverlayRecordName::Whiteout, OverlayXattrPrefix::Trusted) => TRUSTED_OVERLAY_WHITEOUT,
        (OverlayRecordName::Whiteout, OverlayXattrPrefix::User) => USER_OVERLAY_WHITEOUT,
        (OverlayRecordName::Impure, OverlayXattrPrefix::Trusted) => TRUSTED_OVERLAY_IMPURE,
        (OverlayRecordName::Impure, OverlayXattrPrefix::User) => USER_OVERLAY_IMPURE,
        (OverlayRecordName::Uuid, OverlayXattrPrefix::Trusted) => TRUSTED_OVERLAY_UUID,
        (OverlayRecordName::Uuid, OverlayXattrPrefix::User) => USER_OVERLAY_UUID,
    };
    XattrName::try_from_full_name(full_name)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid overlay record xattr name"))
}

/// Named `XattrClass`, not `XattrName`, to avoid clashing with the VFS `XattrName` struct.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum XattrClass {
    Private,
    Passthrough,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) enum OverlayXattrPrefix {
    Trusted,
    User,
}

impl OverlayXattrPrefix {
    pub(in overlayfs) fn as_str(self) -> &'static str {
        match self {
            OverlayXattrPrefix::Trusted => TRUSTED_OVERLAY_PREFIX,
            OverlayXattrPrefix::User => USER_OVERLAY_PREFIX,
        }
    }
}

fn classify(full_name: &str, prefix: OverlayXattrPrefix) -> XattrClass {
    if full_name.starts_with(prefix.as_str()) {
        XattrClass::Private
    } else {
        XattrClass::Passthrough
    }
}

fn is_private(full_name: &str, prefix: OverlayXattrPrefix) -> bool {
    matches!(classify(full_name, prefix), XattrClass::Private)
}

fn used_full_name(name: &XattrName, selected_prefix: &str) -> Result<String> {
    let full_name = name.full_name();
    if !full_name.starts_with(selected_prefix) {
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

/// Callers must consume the returned total, not the buffer contents: entries
/// past a filled buffer are counted but not written.
fn present_xattr_names(
    raw_list: &[u8],
    prefix: OverlayXattrPrefix,
    list_writer: &mut VmWriter,
) -> Result<usize> {
    let selected_prefix = prefix.as_str();
    let mut bytes_written = 0;
    let mut stripped_name = String::new();
    for name_bytes in raw_list.split(|&byte| byte == 0) {
        if name_bytes.is_empty() {
            continue;
        }
        let presented: &[u8] = match core::str::from_utf8(name_bytes) {
            Ok(name) if name.starts_with(selected_prefix) => {
                match name[selected_prefix.len()..].strip_prefix(ESCAPE_INFIX) {
                    Some(stripped_suffix) => {
                        stripped_name.clear();
                        stripped_name.push_str(selected_prefix);
                        stripped_name.push_str(stripped_suffix);
                        stripped_name.as_bytes()
                    }
                    None => continue,
                }
            }
            _ => name_bytes,
        };
        let entry_len = presented.len() + 1;
        if list_writer.avail() == 0 {
            bytes_written += entry_len;
            continue;
        }
        if entry_len > list_writer.avail() {
            return_errno_with_message!(
                Errno::ERANGE,
                "the xattr list buffer is too small for the presented list"
            );
        }
        list_writer.write_fallible(&mut VmReader::from(presented))?;
        list_writer.write_val(&0u8)?;
        bytes_written += entry_len;
    }
    Ok(bytes_written)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MarkerReadSemantics {
    Presence,
    ValueY,
}

pub(super) fn has_marker(
    real_inode: &Arc<dyn Inode>,
    name: XattrName<'static>,
    semantics: MarkerReadSemantics,
) -> Result<bool> {
    let mut value = [0u8; 1];
    let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
    match real_inode.get_xattr(name, &mut writer) {
        Ok(written) => match semantics {
            MarkerReadSemantics::Presence => Ok(true),
            MarkerReadSemantics::ValueY => Ok(written == 1 && value[0] == b'y'),
        },
        Err(err) if err.error() == Errno::ERANGE => {
            Ok(matches!(semantics, MarkerReadSemantics::Presence))
        }
        Err(err) if matches!(err.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => Ok(false),
        Err(err) => Err(err),
    }
}

fn has_impure_marker(real_dir: &Arc<dyn Inode>, prefix: OverlayXattrPrefix) -> Result<bool> {
    has_marker(
        real_dir,
        overlay_record_name(OverlayRecordName::Impure, prefix)?,
        MarkerReadSemantics::Presence,
    )
}

impl OverlayInode {
    /// `real_dentry` is the dentry anchor of `real` — the caller-supplied
    /// half of the `RealObject` anchor invariant (`real_dentry.inode()`
    /// ptr-equals `real`).
    pub(in overlayfs) fn set_overlay_xattr(
        real: &Arc<dyn Inode>,
        real_dentry: &Dentry,
        record: OverlayRecordName,
        prefix: OverlayXattrPrefix,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        let name = overlay_record_name(record, prefix)?;
        real.set_xattr(real_dentry, name, value_reader, flags)
    }

    pub(super) fn set_impure_marker(
        real_dir: &Arc<dyn Inode>,
        real_dentry: &Dentry,
        prefix: OverlayXattrPrefix,
    ) -> Result<()> {
        if has_impure_marker(real_dir, prefix)? {
            return Ok(());
        }
        let mut marker_reader = VmReader::from(IMPURE_MARKER_VALUE).to_fallible();
        Self::set_overlay_xattr(
            real_dir,
            real_dentry,
            OverlayRecordName::Impure,
            prefix,
            &mut marker_reader,
            XattrSetFlags::CREATE_OR_REPLACE,
        )
    }

    fn clear_impure_marker(
        real_dir: &Arc<dyn Inode>,
        real_dentry: &Dentry,
        prefix: OverlayXattrPrefix,
    ) -> Result<()> {
        let name = overlay_record_name(OverlayRecordName::Impure, prefix)?;
        match real_dir.remove_xattr(real_dentry, name) {
            Ok(()) => Ok(()),
            Err(err) if err.error() == Errno::ENODATA => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Clearing assumes lowers are immutable (the overlay never writes them); a
    /// residual race with an external lower writer is deliberately unhandled.
    fn refresh_impure_marker(&self, index: &mut Option<ReaddirIndex>) -> Result<()> {
        let Some(upper_real) = self.upper.get() else {
            return Ok(());
        };
        let prefix = self.fs_arc()?.policy().xattr_prefix();
        if !has_impure_marker(upper_real.real_inode(), prefix)? {
            return Ok(());
        }
        let index = index.as_mut().ok_or_else(|| {
            Error::with_message(Errno::ENOTDIR, "the overlay inode is not a directory")
        })?;
        let facts = self.real_object_stack();
        self.ensure_readdir_index(&facts, index)?;
        let children = index.visible_inodes();
        for child in &children {
            if !child.lowers.is_empty() {
                return Ok(());
            }
        }
        Self::clear_impure_marker(upper_real.real_inode(), upper_real.dentry(), prefix)
    }

    pub(super) fn refresh_impure_marker_best_effort(
        &self,
        index: &mut Option<ReaddirIndex>,
        operation: &'static str,
    ) {
        if let Err(err) = self.refresh_impure_marker(index) {
            warn!(
                "overlay {}: the impure-marker refresh failed (best-effort): {:?}",
                operation, err
            );
        }
    }
}

impl OverlayFs {
    pub(super) fn set_opaque_marker(
        &self,
        temp_path: &Path,
        unsupported_message: &'static str,
    ) -> Result<()> {
        let can_store_private_xattr = self
            .policy()
            .upper_capabilities()
            .is_some_and(|caps| caps.can_store_private_xattr());
        if !can_store_private_xattr {
            return Err(Error::with_message(Errno::EOPNOTSUPP, unsupported_message));
        }
        let mut marker_reader = VmReader::from(OPAQUE_MARKER_VALUE).to_fallible();
        OverlayInode::set_overlay_xattr(
            temp_path.inode(),
            temp_path.dentry(),
            OverlayRecordName::Opaque,
            self.policy().xattr_prefix(),
            &mut marker_reader,
            XattrSetFlags::CREATE_OR_REPLACE,
        )
    }
}

impl OverlayInode {
    pub(super) fn get_xattr_impl(
        &self,
        name: XattrName,
        value_writer: &mut VmWriter,
    ) -> Result<usize> {
        let prefix = self.fs_arc()?.policy().xattr_prefix();
        let used_name = used_full_name(&name, prefix.as_str())?;
        // The `EINVAL` arm is unreachable by construction: the infix is inserted
        // inside the selected namespace, so the escaped name still parses.
        let used = XattrName::try_from_full_name(&used_name).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid escaped overlay xattr name")
        })?;
        self.check_permission(AccessType::ReadOnly, Permission::MAY_READ)?;
        self.delegate_to_real(|real, _d| real.get_xattr(used, value_writer))
    }

    pub(super) fn set_xattr_impl(
        &self,
        self_dentry: &Dentry,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        let prefix = self.fs_arc()?.policy().xattr_prefix();
        let used_name = used_full_name(&name, prefix.as_str())?;
        let used = XattrName::try_from_full_name(&used_name).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid escaped overlay xattr name")
        })?;
        self.check_mutating_permission(
            CopyUpOrigin::Operation(self_dentry),
            Permission::MAY_WRITE,
        )?;
        self.delegate_to_real(|real, d| real.set_xattr(d, used, value_reader, flags))
    }

    pub(super) fn list_xattr_impl(
        &self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        let prefix = self.fs_arc()?.policy().xattr_prefix();
        self.delegate_to_real(|real, _d| {
            let mut raw_list = vec![0u8; XATTR_LIST_MAX_LEN];
            let mut raw_writer = VmWriter::from(&mut raw_list[..]).to_fallible();
            let list_len = real.list_xattr(namespace, &mut raw_writer)?;
            present_xattr_names(&raw_list[..list_len], prefix, list_writer)
        })
    }

    pub(super) fn remove_xattr_impl(&self, self_dentry: &Dentry, name: XattrName) -> Result<()> {
        let prefix = self.fs_arc()?.policy().xattr_prefix();
        let used_name = used_full_name(&name, prefix.as_str())?;
        let used = XattrName::try_from_full_name(&used_name).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid escaped overlay xattr name")
        })?;
        self.check_mutating_permission(
            CopyUpOrigin::Operation(self_dentry),
            Permission::MAY_WRITE,
        )?;
        self.delegate_to_real(|real, d| real.remove_xattr(d, used))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum XattrCopyPolicy {
    /// For the clear-empty recipe: the source directory is about to be deleted,
    /// so xattr-copy errors must never abort it.
    BestEffort,
    /// The copy-up baseline: real errors propagate so no `security.*`/`trusted.*`
    /// xattr is ever silently dropped.
    Strict,
}

fn is_skippable_source_error(err: &Error, copy_policy: XattrCopyPolicy) -> bool {
    copy_policy == XattrCopyPolicy::BestEffort
        || matches!(err.error(), Errno::ENODATA | Errno::ERANGE)
}

impl OverlayInode {
    /// Source reads run under the caller's credentials (no mounter-credential
    /// scope exists), so a denied read propagates rather than silently dropping
    /// `security.*`/`trusted.*` xattrs. Writes land on the workdir temp's own
    /// inode/dentry pair.
    pub(super) fn copy_eligible_xattrs(
        source: &Arc<dyn Inode>,
        temp: &WorkdirTemp,
        copy_policy: XattrCopyPolicy,
        prefix: OverlayXattrPrefix,
    ) -> Result<()> {
        for namespace in [
            XattrNamespace::User,
            XattrNamespace::Trusted,
            XattrNamespace::Security,
        ] {
            let names = match list_xattr_names(source, namespace) {
                Ok(names) => names,
                Err(err) if is_skippable_source_error(&err, copy_policy) => {
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
                if is_private(full_name, prefix) {
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
                let value = match read_xattr_value(source, &name) {
                    Ok(value) => value,
                    Err(err) if is_skippable_source_error(&err, copy_policy) => {
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
                    Err(err) if copy_policy == XattrCopyPolicy::BestEffort => {
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
}

fn list_xattr_names(source: &Arc<dyn Inode>, namespace: XattrNamespace) -> Result<Vec<u8>> {
    let mut probe = VmWriter::from(&mut [] as &mut [u8]).to_fallible();
    let list_len = source.list_xattr(namespace, &mut probe)?;
    let mut names = vec![0u8; list_len];
    let mut list_writer = VmWriter::from(names.as_mut_slice()).to_fallible();
    let written = source.list_xattr(namespace, &mut list_writer)?;
    names.truncate(written);
    Ok(names)
}

fn read_xattr_value(source: &Arc<dyn Inode>, name: &XattrName<'_>) -> Result<Vec<u8>> {
    let reborrow_fn = || match XattrName::try_from_full_name(name.full_name()) {
        Some(name) => name,
        None => unreachable!("the copy loop validated this xattr name"),
    };
    let mut probe = VmWriter::from(&mut [] as &mut [u8]).to_fallible();
    let value_len = source.get_xattr(reborrow_fn(), &mut probe)?;
    let mut value = vec![0u8; value_len];
    let mut value_writer = VmWriter::from(value.as_mut_slice()).to_fallible();
    let written = source.get_xattr(reborrow_fn(), &mut value_writer)?;
    value.truncate(written);
    Ok(value)
}

#[cfg(ktest)]
mod test {
    // SPDX-License-Identifier: MPL-2.0

    //! Unit tests for the pure xattr name mapping: classification, escape,
    //! and the list-present transform.
    //!
    //! The assertions form a fixed case table for those three behaviors. The
    //! tests assert the pure mapping only: no filesystem, VFS, block, or
    //! I/O fixture is constructed.

    use ostd::prelude::ktest;

    use super::*;

    fn xname(full_name: &'static str) -> XattrName<'static> {
        XattrName::try_from_full_name(full_name).unwrap()
    }

    fn own_name_of_len(selected_prefix: &str, full_len: usize) -> String {
        let mut name = String::from(selected_prefix);
        while name.len() < full_len {
            name.push('a');
        }
        name
    }

    fn present_expect_ok(
        raw: &[u8],
        prefix: OverlayXattrPrefix,
        buffer_len: usize,
    ) -> (usize, Vec<u8>) {
        let mut buffer = vec![0u8; buffer_len];
        let mut writer = VmWriter::from(buffer.as_mut_slice()).to_fallible();
        let total = present_xattr_names(raw, prefix, &mut writer).unwrap();
        let written = total.min(buffer_len);
        (total, buffer[..written].to_vec())
    }

    fn present_expect_erange(raw: &[u8], prefix: OverlayXattrPrefix, buffer_len: usize) {
        let mut buffer = vec![0u8; buffer_len];
        let mut writer = VmWriter::from(buffer.as_mut_slice()).to_fallible();
        let err = present_xattr_names(raw, prefix, &mut writer).unwrap_err();
        assert_eq!(err.error(), Errno::ERANGE);
    }

    #[ktest]
    fn classify_private_by_selected_prefix() {
        assert_eq!(
            classify("trusted.overlay.fsz", OverlayXattrPrefix::Trusted),
            XattrClass::Private
        );
        assert_eq!(
            classify("user.overlay.fsz", OverlayXattrPrefix::User),
            XattrClass::Private
        );
        assert_eq!(
            classify("trusted.overlay", OverlayXattrPrefix::Trusted),
            XattrClass::Passthrough
        );
        assert_eq!(
            classify("trusted.overlayfsrz", OverlayXattrPrefix::Trusted),
            XattrClass::Passthrough
        );
        assert_eq!(
            classify("trusted.overlay.", OverlayXattrPrefix::Trusted),
            XattrClass::Private
        );
        assert_eq!(
            classify("Trusted.overlay.x", OverlayXattrPrefix::Trusted),
            XattrClass::Passthrough
        );
        assert_eq!(
            classify("user.overlay.x", OverlayXattrPrefix::Trusted),
            XattrClass::Passthrough
        );
        assert_eq!(
            classify("trusted.overlay.x", OverlayXattrPrefix::User),
            XattrClass::Passthrough
        );
        assert_eq!(
            classify("user.plain.any", OverlayXattrPrefix::Trusted),
            XattrClass::Passthrough
        );
        assert_eq!(
            classify("security.selinux", OverlayXattrPrefix::Trusted),
            XattrClass::Passthrough
        );
        assert_eq!(
            classify("trusted.backup.notes", OverlayXattrPrefix::Trusted),
            XattrClass::Passthrough
        );
        assert!(is_private(
            "trusted.overlay.opaque",
            OverlayXattrPrefix::Trusted
        ));
        assert!(!is_private("user.plain", OverlayXattrPrefix::Trusted));
    }

    #[ktest]
    fn used_full_name_passes_foreign_through() {
        assert_eq!(
            used_full_name(
                &xname("user.plain.any"),
                OverlayXattrPrefix::Trusted.as_str()
            )
            .unwrap(),
            "user.plain.any"
        );
        assert_eq!(
            used_full_name(
                &xname("user.overlay.x"),
                OverlayXattrPrefix::Trusted.as_str()
            )
            .unwrap(),
            "user.overlay.x"
        );
        // The length limit is enforced only on the escape path: a foreign name of
        // length 300 passes through unchanged.
        let foreign = own_name_of_len("user.", 300);
        let used = used_full_name(
            &XattrName::try_from_full_name(foreign.as_str()).unwrap(),
            OverlayXattrPrefix::Trusted.as_str(),
        )
        .unwrap();
        assert_eq!(used, foreign);
    }

    #[ktest]
    fn used_full_name_escapes_own_prefix_unconditionally() {
        assert_eq!(
            used_full_name(
                &xname("trusted.overlay.fsz"),
                OverlayXattrPrefix::Trusted.as_str()
            )
            .unwrap(),
            "trusted.overlay.overlay.fsz"
        );
        assert_eq!(
            used_full_name(
                &xname("trusted.overlay.overlay.fsz"),
                OverlayXattrPrefix::Trusted.as_str()
            )
            .unwrap(),
            "trusted.overlay.overlay.overlay.fsz"
        );
        assert_eq!(
            used_full_name(
                &xname("trusted.overlay."),
                OverlayXattrPrefix::Trusted.as_str()
            )
            .unwrap(),
            "trusted.overlay.overlay."
        );
        assert_eq!(
            used_full_name(
                &xname("user.overlay.fsz"),
                OverlayXattrPrefix::User.as_str()
            )
            .unwrap(),
            "user.overlay.overlay.fsz"
        );
    }

    #[ktest]
    fn used_full_name_enforces_name_length_limit_on_escape() {
        let fits = own_name_of_len(OverlayXattrPrefix::Trusted.as_str(), 247);
        let used = used_full_name(
            &XattrName::try_from_full_name(fits.as_str()).unwrap(),
            OverlayXattrPrefix::Trusted.as_str(),
        )
        .unwrap();
        assert_eq!(used.len(), XATTR_NAME_MAX_LEN);
        let exceeds = own_name_of_len(OverlayXattrPrefix::Trusted.as_str(), 248);
        let err = used_full_name(
            &XattrName::try_from_full_name(exceeds.as_str()).unwrap(),
            OverlayXattrPrefix::Trusted.as_str(),
        )
        .unwrap_err();
        assert_eq!(err.error(), Errno::EOPNOTSUPP);
    }

    #[ktest]
    fn present_strips_one_infix_keeps_prefix() {
        let (total, written) = present_expect_ok(
            b"trusted.overlay.overlay.fsz\0",
            OverlayXattrPrefix::Trusted,
            0,
        );
        assert_eq!(total, 20);
        assert!(written.is_empty());
        let (total, written) = present_expect_ok(
            b"trusted.overlay.overlay.fsz\0",
            OverlayXattrPrefix::Trusted,
            32,
        );
        assert_eq!(total, 20);
        assert_eq!(written, b"trusted.overlay.fsz\0");
        let (_, written) = present_expect_ok(
            b"trusted.overlay.overlay.overlay.x\0",
            OverlayXattrPrefix::Trusted,
            64,
        );
        assert_eq!(written, b"trusted.overlay.overlay.x\0");
        let (_, written) = present_expect_ok(
            b"trusted.overlay.overlay.\0",
            OverlayXattrPrefix::Trusted,
            64,
        );
        assert_eq!(written, b"trusted.overlay.\0");
        let (_, written) =
            present_expect_ok(b"user.overlay.overlay.fsz\0", OverlayXattrPrefix::User, 64);
        assert_eq!(written, b"user.overlay.fsz\0");
        let (_, written) = present_expect_ok(
            b"user.plain\0trusted.overlay.overlay.fsz\0",
            OverlayXattrPrefix::Trusted,
            64,
        );
        assert_eq!(written, b"user.plain\0trusted.overlay.fsz\0");
    }

    #[ktest]
    fn present_hides_own_private_records() {
        for raw in [
            b"trusted.overlay.origin\0".as_slice(),
            b"trusted.overlay.opaque\0".as_slice(),
            b"trusted.overlay.whiteout\0".as_slice(),
            b"trusted.overlay.impure\0".as_slice(),
            b"trusted.overlay.uuid\0".as_slice(),
        ] {
            let (total, written) = present_expect_ok(raw, OverlayXattrPrefix::Trusted, 64);
            assert_eq!(total, 0);
            assert!(written.is_empty());
        }
        let (total, written) = present_expect_ok(
            b"trusted.overlay.no-infix-name\0",
            OverlayXattrPrefix::Trusted,
            64,
        );
        assert_eq!(total, 0);
        assert!(written.is_empty());
        let (total, written) = present_expect_ok(
            b"trusted.overlay.overlay\0",
            OverlayXattrPrefix::Trusted,
            64,
        );
        assert_eq!(total, 0);
        assert!(written.is_empty());
    }

    #[ktest]
    fn present_probe_and_erange_accounting() {
        let (total, written) = present_expect_ok(b"a\0bb\0", OverlayXattrPrefix::Trusted, 0);
        assert_eq!(total, 5);
        assert!(written.is_empty());
        let (total, written) = present_expect_ok(b"a\0bb\0", OverlayXattrPrefix::Trusted, 5);
        assert_eq!(total, 5);
        assert_eq!(written, b"a\0bb\0");
        let (total, written) = present_expect_ok(b"a\0bb\0", OverlayXattrPrefix::Trusted, 2);
        assert_eq!(total, 5);
        assert_eq!(written, b"a\0");
        present_expect_erange(b"a\0bbbb\0", OverlayXattrPrefix::Trusted, 3);
        let (total, written) = present_expect_ok(b"", OverlayXattrPrefix::Trusted, 64);
        assert_eq!(total, 0);
        assert!(written.is_empty());
        let (total, written) = present_expect_ok(b"\0\0", OverlayXattrPrefix::Trusted, 64);
        assert_eq!(total, 0);
        assert!(written.is_empty());
    }

    #[ktest]
    fn present_passes_foreign_and_non_utf8_through() {
        let (_, written) =
            present_expect_ok(b"user.plain\0\xff\xfe\0", OverlayXattrPrefix::Trusted, 64);
        assert_eq!(written, b"user.plain\0\xff\xfe\0");
        let (_, written) = present_expect_ok(
            b"\xfftrusted.overlay.opaque\0",
            OverlayXattrPrefix::Trusted,
            64,
        );
        assert_eq!(written, b"\xfftrusted.overlay.opaque\0");
    }
}
