// SPDX-License-Identifier: MPL-2.0

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

#![short_vis_path::add(overlayfs)]

use super::{OverlayInode, ReaddirCache, copyup::workdir::WorkdirTemp, permission::AccessType};
use crate::{
    fs::{
        file::Permission,
        vfs::{
            inode::Inode,
            path::Dentry,
            xattr::{
                XATTR_LIST_MAX_LEN, XATTR_NAME_MAX_LEN, XattrName, XattrNamespace, XattrSetFlags,
            },
        },
    },
    prelude::*,
};

/// Writing `trusted.*` requires `CAP_SYS_ADMIN`, shielding the overlay's own records from users.
const TRUSTED_OVERLAY_PREFIX: &str = "trusted.overlay.";

/// `user.overlay.` is world-writable, so it cannot shield the overlay's own records.
const USER_OVERLAY_PREFIX: &str = "user.overlay.";

const ESCAPE_INFIX: &str = "overlay.";

/// The shared presence-marker value for `opaque`, `whiteout`, and `impure`; names stay distinct.
pub(super) const MARKER_VALUE: &[u8] = b"y";

/// Returns the private-record prefix for the mount's selected xattr namespace.
fn selected_xattr_prefix(namespace: XattrNamespace) -> &'static str {
    if namespace == XattrNamespace::User {
        USER_OVERLAY_PREFIX
    } else {
        TRUSTED_OVERLAY_PREFIX
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) enum OverlayRecordName {
    Origin,
    Opaque,
    Whiteout,
    Impure,
    Uuid,
}

impl OverlayRecordName {
    /// Builds the xattr name for this record under the mount's selected namespace.
    pub(in overlayfs) fn construct_xattr_name(
        &self,
        namespace: XattrNamespace,
    ) -> Result<XattrName<'static>> {
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
}

/// Maps a caller name to the real name, inserting the escape infix when needed.
fn used_full_name(name: &XattrName, namespace: XattrNamespace) -> Result<String> {
    let selected_prefix = selected_xattr_prefix(namespace);
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

/// Callers must use the returned total: entries past a full buffer are counted, not written.
fn present_xattr_names(
    raw_list: &[u8],
    namespace: XattrNamespace,
    list_writer: &mut VmWriter,
) -> Result<usize> {
    let selected_prefix = selected_xattr_prefix(namespace);
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

fn has_impure_marker(real_dir: &Arc<dyn Inode>, namespace: XattrNamespace) -> Result<bool> {
    has_marker(
        real_dir,
        OverlayRecordName::Impure.construct_xattr_name(namespace)?,
        MarkerReadSemantics::Presence,
    )
}

impl OverlayInode {
    /// `real_dentry` anchors `real`: `real_dentry.inode()` ptr-equals `real`.
    pub(in overlayfs) fn set_overlay_xattr(
        real: &Arc<dyn Inode>,
        real_dentry: &Dentry,
        record: OverlayRecordName,
        namespace: XattrNamespace,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        let name = record.construct_xattr_name(namespace)?;
        real.set_xattr(real_dentry, name, value_reader, flags)
    }

    /// Writes the opacity marker on a real dentry; capability and namespace come from the caller.
    pub(super) fn set_opaque_marker(
        dentry: &Arc<Dentry>,
        namespace: XattrNamespace,
        can_store_private_xattr: bool,
        unsupported_message: &'static str,
    ) -> Result<()> {
        if !can_store_private_xattr {
            return Err(Error::with_message(Errno::EOPNOTSUPP, unsupported_message));
        }
        let mut marker_reader = VmReader::from(MARKER_VALUE).to_fallible();
        Self::set_overlay_xattr(
            dentry.inode(),
            dentry,
            OverlayRecordName::Opaque,
            namespace,
            &mut marker_reader,
            XattrSetFlags::CREATE_OR_REPLACE,
        )
    }

    pub(super) fn set_impure_marker(
        real_dir: &Arc<dyn Inode>,
        real_dentry: &Dentry,
        namespace: XattrNamespace,
    ) -> Result<()> {
        if has_impure_marker(real_dir, namespace)? {
            return Ok(());
        }
        let mut marker_reader = VmReader::from(MARKER_VALUE).to_fallible();
        Self::set_overlay_xattr(
            real_dir,
            real_dentry,
            OverlayRecordName::Impure,
            namespace,
            &mut marker_reader,
            XattrSetFlags::CREATE_OR_REPLACE,
        )
    }

    /// Removes the impure marker xattr, tolerating an already-absent record.
    fn remove_impure_marker(
        real_dir: &Arc<dyn Inode>,
        real_dentry: &Dentry,
        namespace: XattrNamespace,
    ) -> Result<()> {
        let name = OverlayRecordName::Impure.construct_xattr_name(namespace)?;
        match real_dir.remove_xattr(real_dentry, name) {
            Ok(()) => Ok(()),
            Err(err) if err.error() == Errno::ENODATA => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Best-effort clear of the impure marker; assumes no external lower-layer writes.
    pub(super) fn clear_impure_marker(
        &self,
        index: &mut Option<ReaddirCache>,
        operation: &'static str,
    ) {
        let cleared: Result<()> = 'clear: {
            let Some(upper_real) = self.upper.get() else {
                break 'clear Ok(());
            };
            let namespace = self.fs_arc().policy().xattr_namespace();
            match has_impure_marker(upper_real.real_inode(), namespace) {
                Ok(true) => {}
                Ok(false) => break 'clear Ok(()),
                Err(err) => break 'clear Err(err),
            }
            let Some(index) = index.as_mut() else {
                break 'clear Err(Error::with_message(
                    Errno::ENOTDIR,
                    "the overlay inode is not a directory",
                ));
            };
            if let Err(err) = self.ensure_readdir_cache(index) {
                break 'clear Err(err);
            }
            if index.has_impure_entry() {
                break 'clear Ok(());
            }
            Self::remove_impure_marker(upper_real.real_inode(), upper_real.dentry(), namespace)
        };
        if let Err(err) = cleared {
            warn!(
                "overlay {}: the impure-marker refresh failed (best-effort): {:?}",
                operation, err
            );
        }
    }
}

impl OverlayInode {
    pub(super) fn get_xattr_impl(
        &self,
        name: XattrName,
        value_writer: &mut VmWriter,
    ) -> Result<usize> {
        let namespace = self.fs_arc().policy().xattr_namespace();
        let used_name = used_full_name(&name, namespace)?;
        // The `EINVAL` arm is unreachable: the infix is inserted inside the selected namespace.
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
        let namespace = self.fs_arc().policy().xattr_namespace();
        let used_name = used_full_name(&name, namespace)?;
        let used = XattrName::try_from_full_name(&used_name).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid escaped overlay xattr name")
        })?;
        self.check_mutating_permission(self_dentry, Permission::MAY_WRITE)?;
        self.delegate_to_real(|real, d| real.set_xattr(d, used, value_reader, flags))
    }

    pub(super) fn list_xattr_impl(
        &self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        let selected_namespace = self.fs_arc().policy().xattr_namespace();
        self.delegate_to_real(|real, _d| {
            let mut raw_list = vec![0u8; XATTR_LIST_MAX_LEN];
            let mut raw_writer = VmWriter::from(&mut raw_list[..]).to_fallible();
            let list_len = real.list_xattr(namespace, &mut raw_writer)?;
            present_xattr_names(&raw_list[..list_len], selected_namespace, list_writer)
        })
    }

    pub(super) fn remove_xattr_impl(&self, self_dentry: &Dentry, name: XattrName) -> Result<()> {
        let namespace = self.fs_arc().policy().xattr_namespace();
        let used_name = used_full_name(&name, namespace)?;
        let used = XattrName::try_from_full_name(&used_name).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid escaped overlay xattr name")
        })?;
        self.check_mutating_permission(self_dentry, Permission::MAY_WRITE)?;
        self.delegate_to_real(|real, d| real.remove_xattr(d, used))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum XattrCopyPolicy {
    /// Xattr-copy errors must never abort the deletion of the source directory.
    BestEffort,
    /// Real errors propagate so no `security.*`/`trusted.*` xattr is silently dropped.
    Strict,
}

impl XattrCopyPolicy {
    /// Returns whether an xattr error from the source may be skipped for this policy.
    fn can_skip(&self, err: &Error) -> bool {
        *self == XattrCopyPolicy::BestEffort
            || matches!(err.error(), Errno::ENODATA | Errno::ERANGE)
    }
}

impl OverlayInode {
    /// Source reads run under the caller's credentials, so a denied read propagates.
    pub(super) fn copy_eligible_xattrs(
        source: &Arc<dyn Inode>,
        temp: &WorkdirTemp,
        copy_policy: XattrCopyPolicy,
        selected_namespace: XattrNamespace,
    ) -> Result<()> {
        let selected_prefix = selected_xattr_prefix(selected_namespace);
        for namespace in [
            XattrNamespace::User,
            XattrNamespace::Trusted,
            XattrNamespace::Security,
        ] {
            let names = match list_xattr_names(source, namespace) {
                Ok(names) => names,
                Err(err) if copy_policy.can_skip(&err) => {
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
                if full_name.starts_with(selected_prefix) {
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
                    Err(err) if copy_policy.can_skip(&err) => {
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

fn read_xattr_value(source: &Arc<dyn Inode>, name: &XattrName<'_>) -> Result<Vec<u8>> {
    // Re-parse fail-closed: the caller validated the name, but not in the type.
    let reborrow_fn = || {
        XattrName::try_from_full_name(name.full_name()).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "the xattr name is not a valid escaped name")
        })
    };
    let mut probe = VmWriter::from(&mut [] as &mut [u8]).to_fallible();
    let value_len = source.get_xattr(reborrow_fn()?, &mut probe)?;
    let mut value = vec![0u8; value_len];
    let mut value_writer = VmWriter::from(value.as_mut_slice()).to_fallible();
    let written = source.get_xattr(reborrow_fn()?, &mut value_writer)?;
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
        namespace: XattrNamespace,
        buffer_len: usize,
    ) -> (usize, Vec<u8>) {
        let mut buffer = vec![0u8; buffer_len];
        let mut writer = VmWriter::from(buffer.as_mut_slice()).to_fallible();
        let total = present_xattr_names(raw, namespace, &mut writer).unwrap();
        let written = total.min(buffer_len);
        (total, buffer[..written].to_vec())
    }

    fn present_expect_erange(raw: &[u8], namespace: XattrNamespace, buffer_len: usize) {
        let mut buffer = vec![0u8; buffer_len];
        let mut writer = VmWriter::from(buffer.as_mut_slice()).to_fallible();
        let err = present_xattr_names(raw, namespace, &mut writer).unwrap_err();
        assert_eq!(err.error(), Errno::ERANGE);
    }

    #[ktest]
    fn present_selected_namespace_hides_private_records() {
        let hidden = |name: &[u8], namespace: XattrNamespace| {
            let (total, written) = present_expect_ok(name, namespace, 64);
            assert_eq!(total, 0);
            assert!(written.is_empty());
        };
        let shown = |name: &[u8], namespace: XattrNamespace, expected: &[u8]| {
            let (_, written) = present_expect_ok(name, namespace, 64);
            assert_eq!(written, expected);
        };
        hidden(b"trusted.overlay.fsz\0", XattrNamespace::Trusted);
        hidden(b"user.overlay.fsz\0", XattrNamespace::User);
        hidden(b"trusted.overlay.\0", XattrNamespace::Trusted);
        hidden(b"trusted.overlay.opaque\0", XattrNamespace::Trusted);
        shown(
            b"trusted.overlay\0",
            XattrNamespace::Trusted,
            b"trusted.overlay\0",
        );
        shown(
            b"trusted.overlayfsrz\0",
            XattrNamespace::Trusted,
            b"trusted.overlayfsrz\0",
        );
        shown(
            b"Trusted.overlay.x\0",
            XattrNamespace::Trusted,
            b"Trusted.overlay.x\0",
        );
        shown(
            b"user.overlay.x\0",
            XattrNamespace::Trusted,
            b"user.overlay.x\0",
        );
        shown(
            b"trusted.overlay.x\0",
            XattrNamespace::User,
            b"trusted.overlay.x\0",
        );
        shown(
            b"user.plain.any\0",
            XattrNamespace::Trusted,
            b"user.plain.any\0",
        );
        shown(b"user.plain\0", XattrNamespace::Trusted, b"user.plain\0");
        shown(
            b"security.selinux\0",
            XattrNamespace::Trusted,
            b"security.selinux\0",
        );
        shown(
            b"trusted.backup.notes\0",
            XattrNamespace::Trusted,
            b"trusted.backup.notes\0",
        );
    }

    #[ktest]
    fn used_full_name_passes_foreign_through() {
        assert_eq!(
            used_full_name(&xname("user.plain.any"), XattrNamespace::Trusted).unwrap(),
            "user.plain.any"
        );
        assert_eq!(
            used_full_name(&xname("user.overlay.x"), XattrNamespace::Trusted).unwrap(),
            "user.overlay.x"
        );
        // The length limit applies only on the escape path; long foreign names pass unchanged.
        let foreign = own_name_of_len("user.", 300);
        let used = used_full_name(
            &XattrName::try_from_full_name(foreign.as_str()).unwrap(),
            XattrNamespace::Trusted,
        )
        .unwrap();
        assert_eq!(used, foreign);
    }

    #[ktest]
    fn used_full_name_escapes_own_prefix_unconditionally() {
        assert_eq!(
            used_full_name(&xname("trusted.overlay.fsz"), XattrNamespace::Trusted).unwrap(),
            "trusted.overlay.overlay.fsz"
        );
        assert_eq!(
            used_full_name(
                &xname("trusted.overlay.overlay.fsz"),
                XattrNamespace::Trusted
            )
            .unwrap(),
            "trusted.overlay.overlay.overlay.fsz"
        );
        assert_eq!(
            used_full_name(&xname("trusted.overlay."), XattrNamespace::Trusted).unwrap(),
            "trusted.overlay.overlay."
        );
        assert_eq!(
            used_full_name(&xname("user.overlay.fsz"), XattrNamespace::User).unwrap(),
            "user.overlay.overlay.fsz"
        );
    }

    #[ktest]
    fn used_full_name_enforces_name_length_limit_on_escape() {
        let fits = own_name_of_len(selected_xattr_prefix(XattrNamespace::Trusted), 247);
        let used = used_full_name(
            &XattrName::try_from_full_name(fits.as_str()).unwrap(),
            XattrNamespace::Trusted,
        )
        .unwrap();
        assert_eq!(used.len(), XATTR_NAME_MAX_LEN);
        let exceeds = own_name_of_len(selected_xattr_prefix(XattrNamespace::Trusted), 248);
        let err = used_full_name(
            &XattrName::try_from_full_name(exceeds.as_str()).unwrap(),
            XattrNamespace::Trusted,
        )
        .unwrap_err();
        assert_eq!(err.error(), Errno::EOPNOTSUPP);
    }

    #[ktest]
    fn present_strips_one_infix_keeps_prefix() {
        let (total, written) =
            present_expect_ok(b"trusted.overlay.overlay.fsz\0", XattrNamespace::Trusted, 0);
        assert_eq!(total, 20);
        assert!(written.is_empty());
        let (total, written) = present_expect_ok(
            b"trusted.overlay.overlay.fsz\0",
            XattrNamespace::Trusted,
            32,
        );
        assert_eq!(total, 20);
        assert_eq!(written, b"trusted.overlay.fsz\0");
        let (_, written) = present_expect_ok(
            b"trusted.overlay.overlay.overlay.x\0",
            XattrNamespace::Trusted,
            64,
        );
        assert_eq!(written, b"trusted.overlay.overlay.x\0");
        let (_, written) =
            present_expect_ok(b"trusted.overlay.overlay.\0", XattrNamespace::Trusted, 64);
        assert_eq!(written, b"trusted.overlay.\0");
        let (_, written) =
            present_expect_ok(b"user.overlay.overlay.fsz\0", XattrNamespace::User, 64);
        assert_eq!(written, b"user.overlay.fsz\0");
        let (_, written) = present_expect_ok(
            b"user.plain\0trusted.overlay.overlay.fsz\0",
            XattrNamespace::Trusted,
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
            let (total, written) = present_expect_ok(raw, XattrNamespace::Trusted, 64);
            assert_eq!(total, 0);
            assert!(written.is_empty());
        }
        let (total, written) = present_expect_ok(
            b"trusted.overlay.no-infix-name\0",
            XattrNamespace::Trusted,
            64,
        );
        assert_eq!(total, 0);
        assert!(written.is_empty());
        let (total, written) =
            present_expect_ok(b"trusted.overlay.overlay\0", XattrNamespace::Trusted, 64);
        assert_eq!(total, 0);
        assert!(written.is_empty());
    }

    #[ktest]
    fn present_probe_and_erange_accounting() {
        let (total, written) = present_expect_ok(b"a\0bb\0", XattrNamespace::Trusted, 0);
        assert_eq!(total, 5);
        assert!(written.is_empty());
        let (total, written) = present_expect_ok(b"a\0bb\0", XattrNamespace::Trusted, 5);
        assert_eq!(total, 5);
        assert_eq!(written, b"a\0bb\0");
        let (total, written) = present_expect_ok(b"a\0bb\0", XattrNamespace::Trusted, 2);
        assert_eq!(total, 5);
        assert_eq!(written, b"a\0");
        present_expect_erange(b"a\0bbbb\0", XattrNamespace::Trusted, 3);
        let (total, written) = present_expect_ok(b"", XattrNamespace::Trusted, 64);
        assert_eq!(total, 0);
        assert!(written.is_empty());
        let (total, written) = present_expect_ok(b"\0\0", XattrNamespace::Trusted, 64);
        assert_eq!(total, 0);
        assert!(written.is_empty());
    }

    #[ktest]
    fn present_passes_foreign_and_non_utf8_through() {
        let (_, written) =
            present_expect_ok(b"user.plain\0\xff\xfe\0", XattrNamespace::Trusted, 64);
        assert_eq!(written, b"user.plain\0\xff\xfe\0");
        let (_, written) =
            present_expect_ok(b"\xfftrusted.overlay.opaque\0", XattrNamespace::Trusted, 64);
        assert_eq!(written, b"\xfftrusted.overlay.opaque\0");
    }

    #[ktest]
    fn construct_xattr_name_uses_selected_namespace() {
        for (record, trusted, user) in [
            (
                OverlayRecordName::Origin,
                "trusted.overlay.origin",
                "user.overlay.origin",
            ),
            (
                OverlayRecordName::Opaque,
                "trusted.overlay.opaque",
                "user.overlay.opaque",
            ),
            (
                OverlayRecordName::Whiteout,
                "trusted.overlay.whiteout",
                "user.overlay.whiteout",
            ),
            (
                OverlayRecordName::Impure,
                "trusted.overlay.impure",
                "user.overlay.impure",
            ),
            (
                OverlayRecordName::Uuid,
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
}
