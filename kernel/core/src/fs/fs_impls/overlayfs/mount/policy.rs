// SPDX-License-Identifier: MPL-2.0

//! Published mount-policy snapshots.
//!
//! This module owns the immutable [`MountPolicy`] snapshot published by
//! [`OverlayFs`](super::superblock::OverlayFs), the creator-credential policy
//! ([`CreatorCredentialPolicy`]), and the post-claim upper-filesystem
//! capability snapshot ([`UpperFilesystemCapabilities`]).
//! Sibling modules read these published snapshots only; they never re-create,
//! copy ownership of, or mutate them.
//!
//! Construction happens once in `OverlayFs::new` (sibling `build.rs`): the
//! immutable snapshot is assembled by [`MountPolicy::assemble`] after every
//! fallible constituent exists (identity from step 5, capabilities from step
//! 7).
//! `MountPolicy` and `UpperFilesystemCapabilities` are the read-only snapshot
//! the `projection` tree consumes (`is_effective_read_only`,
//! `upper_capabilities`, `can_store_private_xattr`, `can_mknod_char`).

use alloc::format;

use aster_rights::ReadDupOp;

use super::{
    claims::{OVERLAY_UUID_SIZE, OverlayUuid, TRUSTED_OVERLAY_UUID},
    options::{OverlayMountOptions, UuidMode, XinoMode},
};
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        utils::DirentVisitor,
        vfs::{
            inode::{Inode, MknodType},
            path::is_dot_or_dotdot,
            xattr::XattrName,
        },
    },
    prelude::*,
    process::Credentials,
};

/// Prefix of the uniquely-named temporary char-device probe entry created in
/// the workdir staging workspace for the `can_mknod_char` probe.
const CHAR_DEVICE_PROBE_PREFIX: &str = ".overlay-char-device-probe-";

/// Prefix of the uniquely-named temporary file probe entry created in the
/// workdir staging workspace for the d_type probe.
const D_TYPE_PROBE_PREFIX: &str = ".overlay-dtype-probe-";

/// Generates a uniquely-named workdir staging-workspace temp entry for a
/// capability probe.
///
/// Shared by the d_type and char-device probes: one `getrandom` + `format!`
/// sequence instead of two copies (the exact logic is required at two sites
/// within this module).
fn unique_temp_name(prefix: &str) -> String {
    let mut probe_bytes = [0u8; 8];
    crate::util::random::getrandom(&mut probe_bytes);
    format!("{}{:016x}", prefix, u64::from_le_bytes(probe_bytes))
}

/// The immutable, published mount policy snapshot.
///
/// The snapshot is immutable after [`MountPolicy::assemble`] and is the only
/// representation of the mount options/policy: `is_default_permissions` is
/// never duplicated or re-derived (not on `OverlayFs`), and `uuid` is `Some`
/// iff the unified identity is effective.
pub(in crate::fs::fs_impls::overlayfs) struct MountPolicy {
    /// Effective read-only state, fixed before any claim is taken.
    is_effective_read_only: bool,
    /// The UUID/fsid mode.
    #[expect(
        dead_code,
        reason = "the uuid mode policy is not read yet; reserved for the future UUID/fsid policy surface"
    )]
    uuid_mode: UuidMode,
    /// The unified overlay identity; `Some` iff effective.
    uuid: Option<OverlayUuid>,
    /// The stashed creator-credential policy.
    credential_policy: CreatorCredentialPolicy,
    /// The post-claim upper-filesystem capability snapshot.
    upper_capabilities: Option<UpperFilesystemCapabilities>,
    /// Whether the mount was created with the `default_permissions` option.
    is_default_permissions: bool,
    /// The `xino=` option value; single representation, sourced from the
    /// parsed `OverlayMountOptions::xino_mode`.
    xino_mode: XinoMode,
}

impl MountPolicy {
    /// Assembles the immutable policy snapshot.
    ///
    /// The single assembly point; the seven parameters are exactly the
    /// published snapshot's constituents. Called once from `OverlayFs::new`
    /// (sibling `build.rs`) after all fallible constituents exist.
    pub(super) fn assemble(
        is_effective_read_only: bool,
        credential_policy: CreatorCredentialPolicy,
        options: &OverlayMountOptions,
        uuid: Option<OverlayUuid>,
        upper_capabilities: Option<UpperFilesystemCapabilities>,
    ) -> Self {
        Self {
            is_effective_read_only,
            uuid_mode: options.uuid_mode,
            uuid,
            credential_policy,
            upper_capabilities,
            is_default_permissions: options.is_default_permissions,
            xino_mode: options.xino_mode,
        }
    }

    /// Reports the effective read-only state.
    ///
    /// Consumed by `OverlayInode::read_only_gate` from the `projection` tree.
    pub(in crate::fs::fs_impls::overlayfs) fn is_effective_read_only(&self) -> bool {
        self.is_effective_read_only
    }

    /// Reports the `default_permissions` option value.
    ///
    /// Reports the option value only; the permission-stage skip semantics
    /// belong to the `metadata_security` module.
    pub(in crate::fs::fs_impls::overlayfs) fn is_default_permissions(&self) -> bool {
        self.is_default_permissions
    }

    /// Returns the `xino=` mode.
    pub(in crate::fs::fs_impls::overlayfs) fn xino_mode(&self) -> XinoMode {
        self.xino_mode
    }

    /// Returns the effective unified overlay identity, if any.
    ///
    /// `Some` iff the identity is effective; the persisted value is never
    /// changed during the mount lifetime.
    pub(super) fn uuid(&self) -> Option<&OverlayUuid> {
        self.uuid.as_ref()
    }

    /// Returns the stashed creator-credential policy.
    pub(in crate::fs::fs_impls::overlayfs) fn credential_policy(&self) -> &CreatorCredentialPolicy {
        &self.credential_policy
    }

    /// Returns the post-claim upper-filesystem capability snapshot, if this
    /// is a writable mount.
    pub(in crate::fs::fs_impls::overlayfs) fn upper_capabilities(
        &self,
    ) -> Option<&UpperFilesystemCapabilities> {
        self.upper_capabilities.as_ref()
    }
}

/// The creator-credential policy of an overlay mount.
///
/// Stashes the mounting thread's credentials once, at construction, and
/// publishes the scoped-override contract ([`CreatorCredentialPolicy::with_creator_credentials_fn`])
/// that sibling modules use for underlying VFS calls. The credential snapshot
/// is immutable after construction.
pub(in crate::fs::fs_impls::overlayfs) struct CreatorCredentialPolicy {
    /// The stashed creator credentials, taken once at construction from the
    /// current mounting thread's snapshot (`super::with_current_posix_thread`).
    snapshot: Credentials<ReadDupOp>,
    /// The credential source; [`CredentialSource::Creator`] is the only
    /// variant today.
    source: CredentialSource,
}

impl CreatorCredentialPolicy {
    /// Creates the policy from the mounting thread's credential snapshot.
    ///
    /// `build.rs` takes the snapshot once at construction from the current
    /// mounting thread (`super::with_current_posix_thread`).
    pub(super) fn new(snapshot: Credentials<ReadDupOp>) -> Self {
        Self {
            snapshot,
            source: CredentialSource::Creator,
        }
    }

    /// Returns the stashed creator credentials.
    // TODO: Consume this through a VFS API that runs a closure under the stashed credentials.
    #[expect(dead_code, reason = "the VFS has no scoped creator-credential switch")]
    pub(in crate::fs::fs_impls::overlayfs) fn snapshot(&self) -> &Credentials<ReadDupOp> {
        &self.snapshot
    }

    /// Returns the credential source (closed set: `Creator` today).
    #[expect(dead_code, reason = "the VFS has no scoped creator-credential switch")]
    pub(in crate::fs::fs_impls::overlayfs) fn source(&self) -> CredentialSource {
        self.source
    }

    /// Runs `operation_fn` under the stashed creator credentials.
    ///
    /// Scoped execution under the stashed credentials is a VFS dependency: Asterinas
    /// `PosixThread` exposes `credentials()`/`credentials_dup()`/
    /// `credentials_mut()` but no scoped "run with stashed credentials" API,
    /// and `Inode::check_permission` uses `Task::current()` implicitly. Until
    /// that facility lands, `operation_fn` runs with the caller's current
    /// credentials and the stashed snapshot is published for sibling modules
    /// but cannot be installed; no signature is changed.
    ///
    /// TODO: currently a passthrough — callers must not rely on it for
    /// permission decisions; once the VFS API for executing with the
    /// caller's credentials lands, restore the scope switch here.
    pub(in crate::fs::fs_impls::overlayfs) fn with_creator_credentials_fn<T>(
        &self,
        operation_fn: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        operation_fn()
    }
}

/// The source of the credentials used for underlying overlayfs calls.
///
/// Closed set: the mount creator's credentials are always used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) enum CredentialSource {
    /// The credentials of the task that created the mount.
    Creator,
}

/// The post-claim upper-filesystem capability snapshot.
///
/// Immutable after construction; `can_mknod_char` and `can_store_private_xattr`
/// are single-representation probe results that consumers (e.g., the `dir`
/// module's whiteout-representation derivation) never re-probe or re-derive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) struct UpperFilesystemCapabilities {
    /// Whether the upper can store private overlay xattrs (`trusted.`/`user.`
    /// namespace).
    can_store_private_xattr: bool,
    /// Whether the upper reports directory entry types (`d_type` in readdir).
    can_report_directory_type: bool,
    /// Whether the workdir supports the classic whiteout char device `0:0`.
    can_mknod_char: bool,
}

impl UpperFilesystemCapabilities {
    /// Probes the upper/workspace capabilities post-claim.
    ///
    /// Writable mounts only, sleep-capable construction context. The xattr
    /// probe (`get_xattr` on the private overlay namespace) is read-only on
    /// the upper; the d_type probe creates a uniquely-named temporary file in
    /// the workdir staging workspace, scans the workspace until exhausted,
    /// and removes the temp (a workspace entry guarantees a non-vacuous
    /// probe); the `can_mknod_char` probe creates a uniquely-named temporary
    /// char device (`Inode::mknod`, `MknodType::CharDevice(0)`) in the
    /// workdir staging workspace and removes it on success and failure — no
    /// workspace residue. Each probe is a small per-capability helper and the
    /// temp entry names share one [`unique_temp_name`] generator.
    pub(super) fn probe(
        upper_inode: &Arc<dyn Inode>,
        workspace_inode: &Arc<dyn Inode>,
    ) -> Result<Self> {
        let can_store_private_xattr = Self::probe_private_xattr(upper_inode)?;
        let can_report_directory_type = Self::probe_d_type(workspace_inode)?;
        let can_mknod_char = Self::probe_mknod_char(workspace_inode)?;
        Ok(Self {
            can_store_private_xattr,
            can_report_directory_type,
            can_mknod_char,
        })
    }

    /// Probes whether the upper stores private overlay xattrs.
    ///
    /// Read-only on the upper. A backend that answers `ENODATA` (no value yet),
    /// `ERANGE` (a value is present but larger than the probe buffer — itself
    /// positive evidence the private namespace is stored), or returns the
    /// value supports the namespace; `EOPNOTSUPP` means it does not
    /// (fail-closed). Any other error is propagated. `ERANGE` maps to
    /// supported so `UuidMode::Auto` degrades instead of failing on an
    /// over-long foreign value.
    fn probe_private_xattr(upper_inode: &Arc<dyn Inode>) -> Result<bool> {
        let name = XattrName::try_from_full_name(TRUSTED_OVERLAY_UUID).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid overlay xattr probe name")
        })?;
        // The probe buffer is sized at the persisted `trusted.overlay.uuid`
        // value length (`OVERLAY_UUID_SIZE`, 8 bytes): a backend that returns
        // `ERANGE` for a short read (ramfs, ext2) must be counted as
        // supporting the private namespace, not fail the mount.
        let mut value = [0u8; OVERLAY_UUID_SIZE];
        let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        match upper_inode.get_xattr(name, &mut writer) {
            Ok(_) => Ok(true),
            Err(err) if err.error() == Errno::ENODATA => Ok(true),
            Err(err) if err.error() == Errno::ERANGE => Ok(true),
            Err(err) if err.error() == Errno::EOPNOTSUPP => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Probes whether the upper reports directory entry types.
    ///
    /// Probe a directory guaranteed to contain at least one non-dot entry
    /// instead of the usually-empty upper root. A uniquely-named temp file is
    /// created in the workdir staging workspace (the same underlying
    /// filesystem as the upper, enforced by the `container_dev_id` check in
    /// `validate_pair`), the workspace is scanned until exhausted, and the
    /// temp is removed — an empty upper root can no longer make the gate pass
    /// vacuously, and
    /// `InodeType::Unknown` on any non-dot entry is the concrete evidence of a
    /// backend without `d_type` (fail-closed). Residue cleanup is best-effort
    /// on the failure path.
    fn probe_d_type(workspace_inode: &Arc<dyn Inode>) -> Result<bool> {
        let d_type_probe_name = unique_temp_name(D_TYPE_PROBE_PREFIX);
        workspace_inode.create(&d_type_probe_name, InodeType::File, InodeMode::empty())?;
        let mut d_type_probe = DTypeProbeVisitor::new();
        let mut offset = 0;
        let d_type_scan_result = loop {
            match workspace_inode.readdir_at(offset, &mut d_type_probe) {
                Ok(0) => break Ok(()),
                Ok(visited) => offset += visited,
                Err(err) => break Err(err),
            }
        };
        match d_type_scan_result {
            Ok(()) => {
                workspace_inode.unlink(&d_type_probe_name)?;
                Ok(!d_type_probe.saw_unknown_non_dot)
            }
            Err(err) => {
                let _ = workspace_inode.unlink(&d_type_probe_name);
                Err(err)
            }
        }
    }

    /// Probes whether the workdir staging workspace supports the classic
    /// whiteout char device `0:0`.
    ///
    /// The workdir staging workspace hosts a uniquely-named temporary char
    /// device `0:0`. `EOPNOTSUPP` (no classic-whiteout form) and the
    /// permission-class denials `EPERM`/`EACCES` (e.g. a user namespace
    /// without `CAP_MKNOD`, or a host FUSE policy refusing device nodes) all
    /// mean "this backend offers no classic-whiteout form to this mount" and
    /// map to `Ok(false)`, so the whiteout gate falls back to the private-
    /// xattr whiteout form; genuine I/O errors still propagate and fail the
    /// mount. The temp is removed inline on success; only an `unlink` failure
    /// after a successful `mknod` can leave residue, which fails the mount
    /// closed.
    fn probe_mknod_char(workspace_inode: &Arc<dyn Inode>) -> Result<bool> {
        let probe_name = unique_temp_name(CHAR_DEVICE_PROBE_PREFIX);
        match workspace_inode.mknod(&probe_name, InodeMode::empty(), MknodType::CharDevice(0)) {
            Ok(_) => {
                workspace_inode.unlink(&probe_name)?;
                Ok(true)
            }
            Err(err)
                if matches!(
                    err.error(),
                    Errno::EOPNOTSUPP | Errno::EPERM | Errno::EACCES
                ) =>
            {
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }

    /// Reports whether the upper can store private overlay xattrs.
    ///
    /// Consumed by the origin-record store (`projection/lower_id.rs`).
    pub(in crate::fs::fs_impls::overlayfs) fn can_store_private_xattr(&self) -> bool {
        self.can_store_private_xattr
    }

    /// Reports whether the upper reports directory entry types.
    pub(super) fn can_report_directory_type(&self) -> bool {
        self.can_report_directory_type
    }

    /// Reports whether the workdir supports the classic whiteout char device
    /// `0:0`.
    pub(in crate::fs::fs_impls::overlayfs) fn can_mknod_char(&self) -> bool {
        self.can_mknod_char
    }
}

/// A [`DirentVisitor`] that records whether any non-dot entry reports
/// `InodeType::Unknown`.
///
/// Mandated by the `readdir_at` interface (no existing `DirentVisitor`
/// implementation captures entry types), the visitor is the localized shape
/// for the read-only d_type probe of
/// [`UpperFilesystemCapabilities::probe`].
struct DTypeProbeVisitor {
    /// Whether any non-dot entry reported an unknown type.
    saw_unknown_non_dot: bool,
}

impl DTypeProbeVisitor {
    fn new() -> Self {
        Self {
            saw_unknown_non_dot: false,
        }
    }
}

impl DirentVisitor for DTypeProbeVisitor {
    fn visit(&mut self, name: &str, _ino: u64, type_: InodeType, _offset: usize) -> Result<()> {
        if !is_dot_or_dotdot(name) && type_ == InodeType::Unknown {
            self.saw_unknown_non_dot = true;
        }
        Ok(())
    }
}
