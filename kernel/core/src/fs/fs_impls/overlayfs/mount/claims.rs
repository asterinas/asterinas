// SPDX-License-Identifier: MPL-2.0

//! Upper/workdir exclusivity claims and the unified 64-bit overlay identity.
//!
//! This module implements the inode `Extension` runtime lease that carries the claim:
//! each claimed root inode hosts a VFS-owned `OverlayInuseSlot`, and
//! the non-zero unified [`OverlayUuid`] value is both the claim token
//! (per-slot CAS) and, when effective, the overlay UUID persisted as
//! `trusted.overlay.uuid` on the upper root. The upper slot is claimed first
//! and released last; the workdir slot second and released first — enforced
//! structurally by the field declaration order of [`UpperWorkdirClaim`]
//! (`workdir` before `upper`; Rust drops struct fields in declaration order)
//! plus the guard `Drop` order. All claim operations are single-word atomic
//! CASes: non-blocking and safe in `Drop`.

use super::{layers, options::UuidMode};
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        vfs::{
            inode::Inode,
            inode_ext::InodeExt,
            path::{Path, is_dot_or_dotdot},
            xattr::{XattrName, XattrSetFlags},
        },
    },
    prelude::*,
};

/// The size in bytes of the persisted `trusted.overlay.uuid` value.
///
/// `pub(super)` so the sibling `policy.rs` xattr-capability probe sizes its
/// probe buffer at the persisted value length.
pub(super) const OVERLAY_UUID_SIZE: usize = 8;

/// The private xattr name carrying the effective overlay UUID.
///
/// `pub(super)` so the sibling `policy.rs` xattr-capability probe reads the
/// same name the unified identity is persisted in (single representation of
/// the private overlay namespace key).
pub(super) const TRUSTED_OVERLAY_UUID: &str = "trusted.overlay.uuid";

/// The overlay-internal staging workspace name under the workdir root.
///
/// Linux names this directory `work` (`OVL_WORKDIR_NAME` in
/// `fs/overlayfs/super.c`); mount preparation ensures it exists as an empty
/// directory (creating it when absent, recreating it after residue removal)
/// and pins it as the staging workspace. Named constant, no magic string.
const WORKDIR_NAME: &str = "work";

/// The mode of the `<workdir>/work` staging workspace.
///
/// Linux `ovl_workdir_create` creates `work/` with mode 0 (`S_IFDIR|0`,
/// clearing inherited bits) and relies on `generic_permission`'s directory
/// special-case (CAP_DAC_OVERRIDE overrides all DACs for `S_ISDIR`,
/// including the no-exec-bit case), whereas this kernel's
/// `check_permission` applies the "exec override requires at least one exec
/// bit" rule to directories too, so root cannot traverse or unlink inside a
/// mode-0 directory. The workspace therefore uses a usable owner-rwx mode
/// (0o700) instead of replicating 0o000; the test harness's `rm -rf` sweep
/// between runs must also be able to remove leftover staging temps under the
/// workspace.
const WORKDIR_MODE: InodeMode = InodeMode::from_bits_truncate(0o700);

/// The maximum recursion depth of the workdir residue cleanup.
///
/// Directories at `level >= WORKDIR_CLEANUP_MAX_DEPTH` are rmdir'd without
/// descending, so a deeper non-empty directory surfaces the underlying
/// `ENOTEMPTY` instead of unbounded recursion (Linux
/// `ovl_workdir_cleanup`/`ovl_workdir_cleanup_recurse` three-level contract).
const WORKDIR_CLEANUP_MAX_DEPTH: usize = 2;

/// The unified 64-bit identity of one writable overlay mount.
///
/// The value is never zero. It serves as the claim token for both
/// [`InodeClaimGuard`]s (per-`OverlayInuseSlot` CAS) and, when effective, as
/// the overlay UUID persisted as `trusted.overlay.uuid` and published through
/// `MountPolicy::uuid()`/`SuperBlock::fsid`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) struct OverlayUuid(u64);

impl OverlayUuid {
    /// Creates an [`OverlayUuid`], rejecting the zero value with `EINVAL`.
    pub(super) fn try_new(value: u64) -> Result<Self> {
        if value == 0 {
            return_errno_with_message!(Errno::EINVAL, "the overlay uuid must be non-zero");
        }
        Ok(Self(value))
    }

    /// Returns the raw 64-bit value.
    pub(super) fn value(&self) -> u64 {
        self.0
    }

    /// Generates a fresh non-zero identity from the kernel CSPRNG.
    ///
    /// Generation runs pre-claim and lock-free; the zero value has probability
    /// `2^-64` and is rejected by [`OverlayUuid::try_new`], so the loop
    /// regenerates.
    ///
    /// `pub(super)` since the sibling `build.rs` also generates the claim
    /// token directly for effective read-only overlays, where nothing is ever
    /// persisted and a fresh in-memory token suffices.
    pub(super) fn generate() -> Self {
        loop {
            let mut bytes = [0u8; OVERLAY_UUID_SIZE];
            crate::util::random::getrandom(&mut bytes);
            let value = u64::from_le_bytes(bytes);
            if let Ok(uuid) = Self::try_new(value) {
                return uuid;
            }
        }
    }

    /// Reads an existing persisted identity from the upper root (`On`/`Auto`).
    ///
    /// Returns `Ok(None)` when no `trusted.overlay.uuid` xattr exists
    /// (`ENODATA`); a malformed value fails closed with `EINVAL`.
    fn read_from_upper(upper_inode: &Arc<dyn Inode>) -> Result<Option<Self>> {
        let name = XattrName::try_from_full_name(TRUSTED_OVERLAY_UUID)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid overlay uuid xattr name"))?;
        let mut value = [0u8; OVERLAY_UUID_SIZE];
        let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        match upper_inode.get_xattr(name, &mut writer) {
            Ok(written) if written == OVERLAY_UUID_SIZE => {
                Ok(Some(Self::try_new(u64::from_le_bytes(value))?))
            }
            Ok(_) => return_errno_with_message!(
                Errno::EINVAL,
                "the persisted overlay uuid has a malformed value"
            ),
            Err(err) if err.error() == Errno::ENODATA => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Persists the identity as `trusted.overlay.uuid` on the upper root.
    ///
    /// Uses `XattrSetFlags::CREATE_OR_REPLACE` and is only called when the
    /// identity is effective.
    fn persist_on_upper(&self, upper_inode: &Arc<dyn Inode>) -> Result<()> {
        let name = XattrName::try_from_full_name(TRUSTED_OVERLAY_UUID)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid overlay uuid xattr name"))?;
        let value = self.value().to_le_bytes();
        let mut reader = VmReader::from(value.as_slice()).to_fallible();
        upper_inode.set_xattr(name, &mut reader, XattrSetFlags::CREATE_OR_REPLACE)
    }
}

/// A runtime lease on one root inode's `OverlayInuseSlot`.
///
/// The guard pins the claimed inode so the slot cannot be evicted while the
/// claim is held and holds the unified non-zero token. `Drop` re-resolves the
/// slot from the pinned inode and CASes the token free — atomic,
/// non-blocking, safe in `Drop`.
#[derive(Debug)]
pub(super) struct InodeClaimGuard {
    /// Pins the claimed inode (keeps the `OverlayInuseSlot` alive).
    inode: Arc<dyn Inode>,
    /// The unified 64-bit claim token / overlay UUID (non-zero invariant).
    token: OverlayUuid,
}

impl InodeClaimGuard {
    /// Claims the inode's `OverlayInuseSlot` with `identity` as the token.
    ///
    /// Returns `EBUSY` when the slot is already claimed by another holder.
    pub(super) fn try_claim(inode: Arc<dyn Inode>, identity: OverlayUuid) -> Result<Self> {
        inode.overlay_inuse_slot().try_claim(identity.value())?;
        Ok(Self {
            inode,
            token: identity,
        })
    }
}

impl Drop for InodeClaimGuard {
    fn drop(&mut self) {
        // Re-resolve the slot from the pinned inode and CAS the token free.
        // The release is non-blocking and fail-safe: a stale/wrong token is a
        // no-op.
        self.inode.overlay_inuse_slot().release(self.token.value());
    }
}

/// The claimed upper/workdir pair of a writable overlay mount.
///
/// The upper slot is claimed first and released last; the workdir slot second
/// and released first. The field declaration order (`workdir` before `upper`)
/// plus Rust's declaration-order field drops and the guard `Drop` order
/// enforces the release ordering structurally. `identity` is the unified
/// non-zero value used as the token for both slots and, when effective,
/// persisted as `trusted.overlay.uuid`.
#[derive(Debug)]
pub(in crate::fs::fs_impls::overlayfs) struct UpperWorkdirClaim {
    /// Workdir claim; taken second, released first.
    workdir: InodeClaimGuard,
    /// Upper claim; taken first, released last.
    upper: InodeClaimGuard,
    /// Unified identity; persisted iff effective.
    identity: OverlayUuid,
    /// The prepared staging workspace (`<workdir>/work`); `Some` only after
    /// [`prepare_workdir`](Self::prepare_workdir) completed on the writable
    /// branch (Linux `ofs->workdir` dentry-ref parity: staging never
    /// re-resolves the name, and an upper-side unlink of the name does not
    /// invalidate the pinned inode). Written exactly once during mount
    /// construction, before publication; no lock domain. The pinned inode
    /// and its dentry-anchored `Path` travel together (one value), so the
    /// half-prepared state is unrepresentable.
    workdir_workspace: Option<WorkdirWorkspace>,
}

/// The prepared `<workdir>/work` staging workspace — the pinned inode plus
/// its dentry-anchored `Path`.
///
/// Keeps the staging workspace routed through the base VFS dentry layer so
/// every workdir mutation updates the base view's cached directory state.
#[derive(Debug)]
struct WorkdirWorkspace {
    /// The pinned staging workspace inode (`<workdir>/work`).
    inode: Arc<dyn Inode>,
    /// The dentry-anchored staging workspace `Path` (`<workdir>/work`).
    path: Path,
}

impl UpperWorkdirClaim {
    /// Validates the upper/workdir pair structurally.
    ///
    /// Checks that both roots are directories, that they share one mount
    /// node, that they live on the same underlying filesystem (`st_dev`
    /// evidence), and that the workdir is neither identical to nor an
    /// ancestor/descendant of the upperdir. Failures map to `ENOTDIR` /
    /// `EINVAL` (Linux `ovl_fill_super` / `ovl_get_workdir`).
    pub(super) fn validate_pair(upper: &Path, workdir: &Path) -> Result<()> {
        if !upper.type_().is_directory() {
            return_errno_with_message!(Errno::ENOTDIR, "upperdir is not a directory");
        }
        if !workdir.type_().is_directory() {
            return_errno_with_message!(Errno::ENOTDIR, "workdir is not a directory");
        }
        // Linux `ovl_get_workdir` rejects an upper/workdir pair whose roots do
        // not share one mount node (super.c:806-811): the workdir must reside
        // under the same mount as the upperdir, because the later
        // workdir→upper `Path::rename`/`Path::link` operations are
        // same-`Mount` operations (a cross-mount pair would fail with `EXDEV`
        // at operation time).
        if !Arc::ptr_eq(upper.mount_node(), workdir.mount_node()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "workdir and upperdir must reside under the same mount"
            );
        }
        if upper.metadata()?.container_dev_id != workdir.metadata()?.container_dev_id {
            return_errno_with_message!(
                Errno::EINVAL,
                "workdir and upperdir must be on the same underlying filesystem"
            );
        }
        if Arc::ptr_eq(upper.dentry(), workdir.dentry()) {
            return_errno_with_message!(Errno::EINVAL, "workdir must be distinct from upperdir");
        }
        // Alias rejection: two spellings of the same physical directory can
        // produce different `path_name()` strings and different dentry objects
        // (`upperdir=/real/u` with `workdir=/alias/u` where `/alias` is a
        // symlink to `/real`, or a bind-mount alias). The resolved inode
        // objects are the same for the same physical directory, so compare
        // them as well — an aliased same-directory pair must not pass.
        if Arc::ptr_eq(upper.inode(), workdir.inode()) {
            return_errno_with_message!(Errno::EINVAL, "workdir must be distinct from upperdir");
        }

        // Workdir and upperdir must not be each other's ancestor/descendant.
        // The dentry object ancestor chain (`Dentry::is_equal_or_descendant_of`)
        // is reused here — the same predicate as the layer-root overlap
        // validation (`layers.rs`) and the `build.rs` workdir hook — and it
        // respects mount boundaries: parent chains never cross a mount root
        // (a mount root has no parent), so an upper/workdir pair in different
        // mounts is never misjudged as nested (the same-mount check above
        // already requires one mount node). Exact aliases are additionally
        // rejected by the inode-identity check above.
        if workdir.dentry().is_equal_or_descendant_of(upper.dentry())
            || upper.dentry().is_equal_or_descendant_of(workdir.dentry())
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "workdir must not be an ancestor or descendant of upperdir"
            );
        }
        Ok(())
    }

    /// Determines the unified identity before the claim step.
    ///
    /// `On`/`Auto` reuse an existing persisted `trusted.overlay.uuid` when
    /// present; otherwise a fresh non-zero value is generated. `Off`/`Null`
    /// never read and always generate a fresh in-memory-only token. The value
    /// is determined pre-claim because the token must be known at claim time.
    pub(super) fn determine_identity(
        upper_inode: &Arc<dyn Inode>,
        uuid_mode: UuidMode,
    ) -> Result<OverlayUuid> {
        match uuid_mode {
            // `On` fails closed: a backend that cannot serve the xattr read
            // also cannot satisfy persistence, so the read error propagates.
            UuidMode::On => match OverlayUuid::read_from_upper(upper_inode)? {
                Some(existing) => Ok(existing),
                None => Ok(OverlayUuid::generate()),
            },
            // `Auto` degrades on read unavailability: the backend will also
            // fail the post-claim capability probe, so the generated value
            // stays in-memory-only (not effective).
            UuidMode::Auto => match OverlayUuid::read_from_upper(upper_inode) {
                Ok(Some(existing)) => Ok(existing),
                Ok(None) | Err(_) => Ok(OverlayUuid::generate()),
            },
            UuidMode::Off | UuidMode::Null => Ok(OverlayUuid::generate()),
        }
    }

    /// Claims the upper slot first, then the workdir slot.
    ///
    /// On a workdir conflict the already-taken upper claim is dropped
    /// immediately (rollback of the first claim) and the `EBUSY` propagates —
    /// no partial exclusivity escapes construction.
    pub(super) fn claim(
        upper_inode: Arc<dyn Inode>,
        workdir_inode: Arc<dyn Inode>,
        identity: OverlayUuid,
    ) -> Result<Self> {
        let upper = InodeClaimGuard::try_claim(upper_inode, identity)?;
        let workdir = match InodeClaimGuard::try_claim(workdir_inode, identity) {
            Ok(workdir) => workdir,
            Err(err) => {
                // Roll back the first claim before propagating the conflict.
                drop(upper);
                return Err(err);
            }
        };
        Ok(Self {
            workdir,
            upper,
            identity,
            workdir_workspace: None,
        })
    }

    /// Ensures the `<workdir>/work` staging workspace exists, empty, and
    /// pinned.
    ///
    /// The workdir root may contain arbitrary other entries; only the `work`
    /// workspace name (Linux `OVL_WORKDIR_NAME`,
    /// `ovl_make_workdir`/`ovl_workdir_cleanup`) is managed: a non-directory
    /// residue is unlinked, a directory residue is removed depth-first and
    /// rmdir'd, then a fresh empty workspace is created with [`WORKDIR_MODE`]
    /// and pinned on the claim.
    ///
    /// The visible `work` name is removed and recreated through the mount-time
    /// `Path` API (`workdir_path`), so the base view's `DentryChildren` is
    /// updated and the cached directory view stays coherent with the on-disk
    /// removal; Linux `ovl_workdir_cleanup` operates through the upper-fs VFS
    /// dentry layer the same way.
    ///
    /// `workdir_path` is the resolved base-mount workdir root from the
    /// construction sequence (the same object validated and claimed by the
    /// caller). `ENOENT` on the workspace name is a no-op creation step;
    /// every other underlying error propagates unchanged and fails the mount
    /// (fail closed). `ENOTEMPTY` is never returned for residue at the
    /// workdir root; it can surface only as the underlying error of a
    /// genuine residue-subtree cleanup failure. Skipped entirely for
    /// read-only mounts (the caller only invokes it for genuinely writable
    /// overlays). The workspace field is written exactly once here, before
    /// publication; no lock domain.
    pub(super) fn prepare_workdir(&mut self, workdir_path: &Path) -> Result<()> {
        match self.workdir.inode.lookup(WORKDIR_NAME) {
            Ok(residue) if residue.type_().is_directory() => {
                self.remove_work_entries(&residue, 0)?;
                workdir_path.rmdir(WORKDIR_NAME)?;
            }
            Ok(_) => {
                workdir_path.unlink(WORKDIR_NAME)?;
            }
            Err(err) if err.error() == Errno::ENOENT => {}
            Err(err) => return Err(err),
        }
        // The workspace mode diverges from Linux by design:
        // Linux `ovl_workdir_create` uses mode 0 and relies on
        // `generic_permission`'s CAP_DAC_OVERRIDE directory special-case,
        // while this kernel's `check_permission` requires an exec bit to
        // traverse directories even for root; 0o700 keeps the workspace
        // usable and removable by the harness cleanup (see [`WORKDIR_MODE`]).
        let workspace = workdir_path.new_fs_child(WORKDIR_NAME, InodeType::Dir, WORKDIR_MODE)?;
        self.workdir_workspace = Some(WorkdirWorkspace {
            inode: workspace.inode().clone(),
            path: workspace,
        });
        Ok(())
    }

    /// Removes the entries of one residue directory depth-first.
    ///
    /// `level` is the recursion depth from the residue root (`0`); directories
    /// at `level >= 2` are rmdir'd without descending, so a deeper non-empty
    /// directory surfaces the underlying `ENOTEMPTY` instead of unbounded
    /// recursion (Linux `ovl_workdir_cleanup`/`ovl_workdir_cleanup_recurse`
    /// three-level contract).
    fn remove_work_entries(&self, dir: &Arc<dyn Inode>, level: usize) -> Result<()> {
        // `readdir_at` is a batched interface (the underlying backend returns
        // one batch per call and a continuation cookie); a single call can
        // therefore leave residue beyond the first batch, and a later
        // `rmdir(WORKDIR_NAME)` would surface the underlying `ENOTEMPTY` and
        // fail the mount. Loop with the returned offset until `Ok(0)` drains
        // the directory (the same discipline as `probe_d_type` in
        // `mount/policy.rs` and the standard `InodeHandle::readdir` caller);
        // the `Vec<String>` visitor appends across batches.
        let mut names: Vec<String> = Vec::new();
        let mut offset = 0;
        loop {
            match dir.readdir_at(offset, &mut names)? {
                0 => break,
                visited => offset += visited,
            }
        }
        names.retain(|name| !is_dot_or_dotdot(name));
        for name in names {
            let child = dir.lookup(&name)?;
            if child.type_().is_directory() {
                if level < WORKDIR_CLEANUP_MAX_DEPTH {
                    self.remove_work_entries(&child, level + 1)?;
                }
                dir.rmdir(&name)?;
            } else {
                dir.unlink(&name)?;
            }
        }
        Ok(())
    }

    /// Persists the unified identity as `trusted.overlay.uuid`.
    ///
    /// Called only when the identity is effective (after the capability gates
    /// pass and persistence is decided). The caller (`build.rs`) maps an `On` persist failure to
    /// `EOPNOTSUPP` fail-closed and an `Auto` persist failure to degrade to
    /// not-effective.
    pub(super) fn persist_identity(&self) -> Result<()> {
        self.identity.persist_on_upper(&self.upper.inode)
    }

    /// Returns the pinned staging workspace inode.
    ///
    /// `Ok` only after [`prepare_workdir`](Self::prepare_workdir) ran;
    /// `Err(EROFS)` when the workspace was never prepared (upper-backed
    /// effective read-only mount) — the claim exists but staging must still
    /// fail closed.
    pub(in crate::fs::fs_impls::overlayfs) fn workdir_workspace(&self) -> Result<&Arc<dyn Inode>> {
        self.workdir_workspace
            .as_ref()
            .map(|workspace| &workspace.inode)
            .ok_or_else(|| {
                Error::with_message(
                    Errno::EROFS,
                    "the overlay workdir workspace is not prepared",
                )
            })
    }

    /// Returns the dentry-anchored staging workspace path.
    ///
    /// `Ok` only after [`prepare_workdir`](Self::prepare_workdir) ran;
    /// `Err(EROFS)` when the workspace was never prepared (upper-backed
    /// effective read-only mount) — the claim exists but staging must still
    /// fail closed.
    pub(in crate::fs::fs_impls::overlayfs) fn workdir_workspace_path(&self) -> Result<&Path> {
        self.workdir_workspace
            .as_ref()
            .map(|workspace| &workspace.path)
            .ok_or_else(|| {
                Error::with_message(
                    Errno::EROFS,
                    "the overlay workdir workspace is not prepared",
                )
            })
    }
}

/// Probes that a root path resolves to a backend-instance-stable inode.
///
/// Resolving the same root path twice must yield `Arc::ptr_eq`-equal inodes,
/// proving the backend's inode cache is instance-stable for pinned roots. In
/// addition, both resolutions must be the same instance as `pinned_inode` —
/// the layer-pinned object that `UpperWorkdirClaim::claim` actually claims — so the checked
/// object and the used object are the same. This is a heuristic; the durable
/// guarantee is the backend identity contract. A failing backend fails closed
/// with `EOPNOTSUPP`.
pub(super) fn verify_inode_instance_stability(
    raw_path: &str,
    pinned_inode: &Arc<dyn Inode>,
) -> Result<()> {
    // Both resolutions go through the shared `layers::resolve_root_path` helper; each
    // resolution is compared both to the other and to the layer-pinned inode
    // that is claimed downstream.
    let first = layers::resolve_root_path(raw_path)?.inode().clone();
    let second = layers::resolve_root_path(raw_path)?.inode().clone();
    if !Arc::ptr_eq(&first, &second) || !Arc::ptr_eq(&first, pinned_inode) {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "the underlying filesystem does not provide instance-stable inodes for pinned roots"
        );
    }
    Ok(())
}
