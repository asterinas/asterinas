// SPDX-License-Identifier: MPL-2.0

//! Layer stack assembly for the overlay filesystem.
//!
//! This module resolves the real `upperdir`/`lowerdir` roots into pinned
//! [`OverlayLayer`]s and freezes them into an immutable [`OverlayLayerStack`].
//! It owns layer-root resolution, layer ordering, the per-unique-underlying-
//! superblock `fsid` assignment, and the layer-root overlap validation.
//! The stack is constructed once by
//! [`OverlayLayerStack::assemble`] during `OverlayFs::new` and is immutable
//! afterwards for the mount lifetime; sibling modules read it only.
//!
//! Lower-layer writability: the overlay never writes the lower layers.
//! The guarantee holds in two parts —
//! for non-`default_permissions` mounts mutating paths promote to the
//! upper first; for
//! `default_permissions` mounts it is completed by a pending
//! permission-boundary fix, until which that configuration carries a
//! known defect. External concurrent modification of the lower
//! layers is an unsupported operation (documented): the projection and
//! identity logic rely on the layer stack being stable for the mount
//! lifetime, and an external lower writer can corrupt the visible merge
//! (e.g. the `refresh_impure_marker` check-use race). The mount boundary
//! therefore rejects the one truly mountable corruption form — lower/
//! upper/workdir/lower-root overlap — while read-write lower backends
//! remain accepted (Linux overlayfs parity).

use device_id::DeviceId;

use crate::{
    fs::vfs::{
        file_system::{FileSystem, FsFlags},
        inode::Inode,
        path::{AT_FDCWD, Dentry, EmptyPathStr, FsPath, Mount, Path},
    },
    prelude::*,
};

/// Resolves `raw_path` through `lookup_no_follow` in the mounting task's
/// filesystem context.
///
/// This is the single shared path-resolution helper of the mount module:
/// the [`OverlayLayerStack::assemble`] layer-root resolution, the sibling
/// `build.rs` upper/workdir resolution, and the `claims.rs`
/// instance-stability probe all go through this helper instead of each
/// re-implementing the `FsPath::from_fd_at(AT_FDCWD, …)` +
/// `resolver().read().lookup_no_follow(…)` sequence (the exact logic is
/// required at multiple sites within this module). Intermediate symlink
/// components are followed; the final component is not (mount-time roots are
/// the literal resolved directories).
///
/// A 4-tuple keeps the two-phase assembly shape (resolve-then-assign) and
/// avoids defining a dedicated intermediate type; the tuple is consumed only inside
/// [`OverlayLayerStack::assemble`] and never crosses a public boundary.
type LayerParts = (RealPath, Arc<dyn Inode>, Arc<dyn FileSystem>, DeviceId);

pub(super) fn resolve_root_path(raw_path: &str) -> Result<Path> {
    let fs_path = FsPath::from_fd_at(AT_FDCWD, raw_path, EmptyPathStr::Reject)?;
    // Resolve inside a single statement so the `read_fs()` read guard and
    // the resolver read guard live exactly as long as the lookup (same
    // shape as `resolver.rs::lookup` current-thread pattern); neither
    // escapes this scope.
    super::with_current_posix_thread(|posix_thread| {
        let fs = posix_thread.read_fs();
        fs.resolver().read().lookup_no_follow(&fs_path)
    })
}

/// The ordered, immutable layer stack of an overlay mount.
///
/// Lookup searches the upper layer first (when present) and then the lower
/// layers top-to-bottom. The stack is assembled exactly once and is immutable
/// after construction; sibling modules read it only and never re-create, copy
/// ownership of, or mutate it.
#[derive(Debug)]
pub(in crate::fs::fs_impls::overlayfs) struct OverlayLayerStack {
    /// Pinned upper layer; present iff the overlay is writable.
    pub(in crate::fs::fs_impls::overlayfs) upper: Option<OverlayLayer>,
    /// Pinned lower layers; non-empty and ordered topmost-first.
    pub(in crate::fs::fs_impls::overlayfs) lowers: Vec<OverlayLayer>,
}

/// A dentry-anchored real path whose anchor mount is held weakly.
///
/// The stored paths of an overlay mount — `OverlayLayer.root_path`
/// and `RealObject.real_path` — must never pin the parent overlay's
/// `Mount`/`OverlayFs` lifetime (a fix for upstream xfstests overlay/029): a stored path
/// surviving teardown would otherwise keep the parent's claim guards from
/// releasing on the final `Drop`. `RealPath` therefore holds the anchor
/// mount weakly (`Weak<Mount>`), alongside the dentry anchor (strong pin: a
/// `Dentry` holds no `Mount` reference, so the dentry chain cannot keep the
/// mount alive) and the real inode of the dentry anchor (strong pin, derived
/// once at construction from the live path so the inode and the path always
/// refer to the same dentry-layer object). The anchor is upgraded per use by
/// [`RealPath::upgrade`]; a dead anchor fails closed with `Errno::EIO`.
#[derive(Clone, Debug)]
pub(in crate::fs::fs_impls::overlayfs) struct RealPath {
    /// The anchor mount; held weakly so a surviving stored path cannot pin it.
    /// Upgraded per use by [`RealPath::upgrade`].
    mount: Weak<Mount>,
    /// The dentry anchor within the anchor mount (strong pin: the dentry
    /// chain and its inodes stay alive while this stored path lives; a `Dentry`
    /// holds no `Mount` reference, so this pin cannot keep the mount alive).
    dentry: Arc<Dentry>,
    /// The real inode of the dentry anchor (strong pin; derived once at
    /// construction from the live path so the inode and the path always
    /// refer to the same dentry-layer object).
    inode: Arc<dyn Inode>,
}

impl RealPath {
    /// Builds the stored path from a live, dentry-anchored path, downgrading the
    /// anchor mount.
    ///
    /// The single construction path; enforces the "inode/path/dentry refer
    /// to the same dentry-layer object" contract at one site. The stored path
    /// pins the dentry chain and the real inode but never the anchor mount.
    pub(in crate::fs::fs_impls::overlayfs) fn from_path(path: &Path) -> Self {
        Self {
            mount: Arc::downgrade(path.mount_node()),
            dentry: path.dentry().clone(),
            inode: path.inode().clone(),
        }
    }

    /// Upgrades the weak anchor mount into a live `Path`.
    ///
    /// Returns `Err(EIO)` when the anchor mount is no longer alive (the
    /// parent overlay was unmounted while a stored path survived); no
    /// namespace-mutating or dentry-routed operation may proceed on a dead
    /// anchor. Lock-free atomic `Weak::upgrade`; adds no lock edge and never
    /// crosses a `Bio` boundary.
    pub(in crate::fs::fs_impls::overlayfs) fn upgrade(&self) -> Result<Path> {
        let mount = self.mount.upgrade().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the anchor mount of the stored real path is no longer alive",
            )
        })?;
        Ok(Path::new(mount, self.dentry.clone()))
    }

    /// Returns the pinned real inode without upgrading the mount (infallible).
    pub(in crate::fs::fs_impls::overlayfs) fn inode(&self) -> &Arc<dyn Inode> {
        &self.inode
    }
}

/// One pinned real layer root of an overlay mount.
///
/// The pins keep the underlying layer roots alive for the mount lifetime:
/// the dentry-anchored [`RealPath`] anchor and the resolved root inode are
/// both captured at mount and never re-resolved by string afterwards.
/// `container_dev_id` carries the `st_dev` same-filesystem evidence used by
/// the upper/workdir validation.
#[derive(Debug)]
pub(in crate::fs::fs_impls::overlayfs) struct OverlayLayer {
    /// Dentry-anchored layer-root anchor resolved at mount (Linux
    /// `ovl_path_upper`/`ovl_dentry_upper` dentry-ref parity: the layer stack
    /// pins the base-mount root dentry for the mount lifetime, and every
    /// derived real-object path stays rooted on this anchor). The anchor
    /// mount is held weakly ([`RealPath`]), so a surviving layer stack cannot
    /// pin the parent overlay's `Mount`/`OverlayFs` lifetime after unmount;
    /// the layer's `root_inode`/`fs` strong pins keep the layer root and its
    /// underlying filesystem alive while the layer lives.
    pub(in crate::fs::fs_impls::overlayfs) root_path: RealPath,
    /// Pinned real layer root (lifetime pin).
    pub(in crate::fs::fs_impls::overlayfs) root_inode: Arc<dyn Inode>,
    /// Underlying filesystem identity of the layer root.
    pub(in crate::fs::fs_impls::overlayfs) fs: Arc<dyn FileSystem>,
    /// Per-unique-underlying-superblock identifier assigned at assembly.
    ///
    /// Immutable after construction: [`OverlayLayerStack::assemble`] resolves
    /// every layer root first, computes the final identifier from the unique
    /// underlying filesystem instances, and only then constructs the layer
    /// with its final value — no placeholder `fsid` state is ever published
    /// and no post-construction `&mut` rewrite exists.
    pub(in crate::fs::fs_impls::overlayfs) fsid: u64,
    /// `st_dev` of the layer root, used for same-filesystem comparisons.
    pub(in crate::fs::fs_impls::overlayfs) container_dev_id: DeviceId,
}

impl OverlayLayer {
    /// Resolves `raw_path` into the pinned layer-root parts.
    ///
    /// The two-phase assembly order: [`OverlayLayerStack::assemble`]
    /// resolves every raw path into its parts `(root_path, root_inode, fs,
    /// container_dev_id)` first, computes the per-unique-underlying-superblock
    /// `fsid` mapping from those parts, and only then constructs the final
    /// [`OverlayLayer`]s with their final identifiers. This resolution phase
    /// is the shared upper/lower construction step: it resolves with
    /// `lookup_no_follow` in the mounting task's filesystem context (a
    /// missing path surfaces the resolver's `ENOENT` and a non-directory root
    /// fails with `ENOTDIR`), pins the resolved inode and filesystem for the
    /// mount lifetime, and downgrades the resolved `Path` into the
    /// layer-root anchor [`RealPath`] (`root_path`).
    fn resolve_parts(raw_path: &str) -> Result<LayerParts> {
        let path = resolve_root_path(raw_path)?;
        if !path.type_().is_directory() {
            return_errno_with_message!(Errno::ENOTDIR, "the layer root is not a directory");
        }
        Ok((
            RealPath::from_path(&path),
            path.inode().clone(),
            path.fs(),
            path.metadata()?.container_dev_id,
        ))
    }

    /// Rejects an overlap between `new` and every already-assembled layer
    /// root in `others` (Linux `ovl_check_overlapping_layers` parity).
    ///
    /// Two roots overlap when they resolve to the same directory — either the
    /// same dentry object or the same inode object (two spellings of the same
    /// physical directory, e.g. a symlink or bind-mount alias; the inode
    /// identity is instance-stable for pinned roots) — or when one is an
    /// ancestor/descendant of the other in the resolved hierarchy (the
    /// dentry object ancestor chain, [`Dentry::is_equal_or_descendant_of`],
    /// shared with [`super::claims::UpperWorkdirClaim::validate_pair`] and
    /// the sibling `build.rs` workdir hook). The object chain naturally
    /// respects mount boundaries: parent chains never cross a mount root (a
    /// mount root has no parent), so a layer root that is another mount's
    /// root is neither an ancestor nor a descendant of anything in that mount
    /// (lowerdir = a nested overlay's mount root is legal, overlay/029).
    /// Violations return `EINVAL`. Only the layer roots themselves are
    /// compared, so legal nested subdirectories (a lower tree that merely
    /// contains the upper's parent directory, the normal deployment shape)
    /// are never rejected. The workdir is not a layer and is covered by the
    /// same predicate through the sibling `build.rs` hook.
    fn validate_layer_overlap(new: &OverlayLayer, others: &[&OverlayLayer]) -> Result<()> {
        let new_path = new.root_path.upgrade()?;
        let new_dentry = new_path.dentry();
        for other in others {
            let other_path = other.root_path.upgrade()?;
            let other_dentry = other_path.dentry();
            if Arc::ptr_eq(new_dentry, other_dentry)
                || Arc::ptr_eq(&new.root_inode, &other.root_inode)
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "overlay layer roots must be distinct directories"
                );
            }
            if new_dentry.is_equal_or_descendant_of(other_dentry)
                || other_dentry.is_equal_or_descendant_of(new_dentry)
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "overlay layer roots must not be each other's ancestor or descendant"
                );
            }
        }
        Ok(())
    }
}

impl OverlayLayerStack {
    /// Assembles the resolved upper/lower layer stack of an overlay mount.
    ///
    /// `upper_dir` is present only when a writable overlay was requested;
    /// `lower_dirs` carries the option-order lower paths; the first option is
    /// the topmost lower layer (Linux `lowerdir=/l1:/l2:/l3` stacks `l1`
    /// topmost). The upper root fails with `EROFS` when its backend reports
    /// `FsFlags::RDONLY` and the overlay itself was not forced read-only;
    /// `is_forced_read_only` is the already-parsed option value fed from
    /// `OverlayMountOptions`. Every layer root is resolved through
    /// [`OverlayLayer::resolve_parts`] and one `fsid` is assigned per unique
    /// underlying filesystem instance, deduplicated at assembly time.
    ///
    /// Two-phase construction: all raw paths are resolved into their
    /// pinned parts first, the unique-filesystem `fsid` mapping is computed
    /// from those parts, and only then are the final [`OverlayLayer`]s
    /// constructed with their final identifiers — no placeholder `fsid`
    /// state is ever published, so the published fields are immutable after
    /// construction. The overlap validation runs after construction
    /// and before the stack is returned: the upper (when present) and every
    /// lower pair must be distinct directories that are neither each
    /// other's ancestor nor descendant.
    ///
    /// Non-empty lower layers are enforced at this checked constructor: an
    /// empty `lower_dirs` is rejected with `EINVAL` instead of being admitted
    /// by a `Vec` that documents the invariant in a comment only.
    pub(super) fn assemble(
        upper_dir: Option<String>,
        lower_dirs: Vec<String>,
        is_forced_read_only: bool,
    ) -> Result<Self> {
        let mut upper_parts = None;
        if let Some(raw_path) = upper_dir {
            let (root_path, root_inode, fs, container_dev_id) =
                OverlayLayer::resolve_parts(&raw_path)?;
            // A writable overlay cannot be served by a read-only upper
            // backend unless the overlay itself was forced read-only.
            if !is_forced_read_only && fs.flags().contains(FsFlags::RDONLY) {
                return_errno_with_message!(Errno::EROFS, "the upper filesystem is read-only");
            }
            upper_parts = Some((root_path, root_inode, fs, container_dev_id));
        }

        // Defensive structural rejection of the illegal empty state:
        // `OverlayMountOptions::parse` guarantees a non-empty `lowerdir`, but
        // the checked constructor is the last line of defense so the
        // published stack can never carry an empty `lowers` vector.
        if lower_dirs.is_empty() {
            return_errno_with_message!(
                Errno::EINVAL,
                "at least one lower layer is required to assemble the layer stack"
            );
        }
        let lower_parts: Vec<LayerParts> = lower_dirs
            .iter()
            .map(|raw_path| OverlayLayer::resolve_parts(raw_path))
            .collect::<Result<_>>()?;

        // Assign one `fsid` per unique underlying superblock: layers pinned
        // on the same underlying filesystem instance share a single
        // identifier. The identifier is assigned in stack order (upper first
        // when present), so the upper filesystem always owns `fsid` 0 on
        // writable overlays, mirroring the Linux `ovl_layer[].fsid` layout.
        // The assignments complete inside `assemble`, so the published stack
        // is still immutable after construction.
        let mut unique_fses: Vec<Arc<dyn FileSystem>> = Vec::new();
        let mut fsid_of_fn = |fs: &Arc<dyn FileSystem>| -> u64 {
            if let Some(index) = unique_fses
                .iter()
                .position(|seen_fs| Arc::ptr_eq(seen_fs, fs))
            {
                index as u64
            } else {
                unique_fses.push(fs.clone());
                (unique_fses.len() - 1) as u64
            }
        };

        // Construct the final layers with their final `fsid` (upper first
        // when present, then lowers topmost-first); the `fsid` field is
        // immutable after this construction (no `&mut` rewrite exists).
        let upper = upper_parts.map(|(root_path, root_inode, fs, container_dev_id)| {
            let fsid = fsid_of_fn(&fs);
            OverlayLayer {
                root_path,
                root_inode,
                fs,
                fsid,
                container_dev_id,
            }
        });
        let lowers = lower_parts
            .into_iter()
            .map(|(root_path, root_inode, fs, container_dev_id)| {
                let fsid = fsid_of_fn(&fs);
                OverlayLayer {
                    root_path,
                    root_inode,
                    fs,
                    fsid,
                    container_dev_id,
                }
            })
            .collect::<Vec<_>>();

        // Pairwise overlap validation of the upper (when present) and
        // every lower root. The workdir is not a layer and is covered by the
        // sibling `build.rs` hook using the same predicate.
        let all_layers: Vec<&OverlayLayer> = upper.iter().chain(lowers.iter()).collect();
        for (index, new_layer) in all_layers.iter().enumerate() {
            OverlayLayer::validate_layer_overlap(new_layer, &all_layers[index + 1..])?;
        }

        Ok(Self { upper, lowers })
    }
}
