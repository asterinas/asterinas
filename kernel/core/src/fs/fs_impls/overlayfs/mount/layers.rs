// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Layer stack assembly for the overlay filesystem.
//!
//! This module resolves the real `upperdir`/`lowerdir` roots into pinned
//! [`Layer`]s and freezes them into a [`LayerStack`]. It owns
//! layer-root resolution, layer ordering, the per-unique-underlying-superblock
//! `fsid` assignment, and the layer-root overlap validation. The stack is
//! constructed once by [`LayerStack::assemble`] during `OverlayFs::new`.
//!
//! Lower layers are read-only: the overlay never writes the lower layers.
//!
//! - Non-`default_permissions` mounts promote mutating paths to the upper
//!   first.
//! - `default_permissions` mounts keep a documented limitation: the persisted
//!   directory-merging staleness marker (the overlay `trusted.overlay.impure`
//!   xattr) is not refreshed after mutations, so the marker can remain stale.
//!   This limitation is scoped to that persisted marker; the other layer-stack
//!   invariants in this module still hold.
//! - External concurrent modification of the lower layers is unsupported:
//!   projection and identity assume a stable layer stack, and an external
//!   lower writer can corrupt the visible merge.
//! - The mount boundary rejects the one mountable corruption form — lower/
//!   upper/workdir/lower-root overlap — while read-write lower backends
//!   remain accepted.
//!
//! References:
//!
//! - <https://elixir.bootlin.com/linux/v7.0/source/Documentation/filesystems/overlayfs.rst#L350-L364>
//!   (Linux overlayfs parity; stacks colon-separated lowerdirs with the first entry topmost)
//! - <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/super.c#L1273>
//!   (Linux `ovl_check_overlapping_layers`)
//! - <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/ovl_entry.h#L33-L42>
//!   (Linux `ovl_layer[].fsid`, upper fsid 0)

use device_id::DeviceId;

use crate::{
    fs::{
        fs_impls::overlayfs::projection::{LowerLayerIdentity, RealObject},
        vfs::{
            file_system::{FileSystem, FsFlags},
            inode::Inode,
            path::{AT_FDCWD, Dentry, EmptyPathStr, FsPath, Mount, Path},
        },
    },
    prelude::*,
};

/// Two-phase assembly input: resolve-then-assign.
type LayerParts = (RealPath, Arc<dyn FileSystem>, DeviceId);

/// Resolves `raw_path` through `lookup_no_follow` in the mounting task's
/// filesystem context: intermediate symlink components are followed, the
/// final component is not (mount-time roots are the literal resolved
/// directories). This is the single shared path-resolution helper of the
/// mount module, used for the upper/workdir resolution and the
/// instance-stability probe.
pub(super) fn resolve_root_path(raw_path: &str) -> Result<Path> {
    let fs_path = FsPath::from_fd_at(AT_FDCWD, raw_path, EmptyPathStr::Reject)?;
    super::super::with_current_posix_thread(|_task, posix_thread| {
        let fs = posix_thread.read_fs();
        fs.resolver().read().lookup_no_follow(&fs_path)
    })
    .ok_or_else(|| {
        Error::with_message(
            Errno::EINVAL,
            "the overlay mount has no current task or POSIX thread",
        )
    })?
}

/// Probes that a root path resolves to a backend-instance-stable inode.
///
/// Both resolutions must match `pinned_inode`, so the checked object is the
/// one that [`super::claims::UpperWorkdirClaim::claim`] later uses. This is a
/// heuristic; a failing backend returns `EOPNOTSUPP`.
pub(super) fn verify_inode_instance_stability(
    raw_path: &str,
    pinned_inode: &Arc<dyn Inode>,
) -> Result<()> {
    let first = resolve_root_path(raw_path)?.inode().clone();
    let second = resolve_root_path(raw_path)?.inode().clone();
    if !Arc::ptr_eq(&first, &second) || !Arc::ptr_eq(&first, pinned_inode) {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "the underlying filesystem does not provide instance-stable inodes for pinned roots"
        );
    }
    Ok(())
}

/// The ordered, immutable layer stack of an overlay mount.
#[derive(Debug)]
pub(in overlayfs) struct LayerStack {
    pub(super) upper: Option<Layer>,
    pub(super) lowers: Vec<Layer>,
}

#[derive(Clone, Debug)]
pub(in overlayfs) struct RealPath {
    mount: Weak<Mount>,
    dentry: Arc<Dentry>,
    inode: Arc<dyn Inode>,
}

impl RealPath {
    pub(in overlayfs) fn from_path(path: &Path) -> Self {
        Self {
            mount: Arc::downgrade(path.mount_node()),
            dentry: path.dentry().clone(),
            inode: path.inode().clone(),
        }
    }

    /// Returns `Err(EIO)` when the anchor mount is no longer alive (the
    /// parent overlay was unmounted while a stored path survived).
    pub(in overlayfs) fn upgrade(&self) -> Result<Path> {
        let mount = self.mount.upgrade().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the anchor mount of the stored real path is no longer alive",
            )
        })?;
        Ok(Path::new(mount, self.dentry.clone()))
    }

    pub(in overlayfs) fn inode(&self) -> &Arc<dyn Inode> {
        &self.inode
    }
}

/// One pinned real layer root of an overlay mount.
#[derive(Debug)]
pub(in overlayfs) struct Layer {
    pub(in overlayfs) root_path: RealPath,
    pub(in overlayfs) fs: Arc<dyn FileSystem>,
    /// Per-unique-underlying-superblock identifier assigned at assembly.
    pub(in overlayfs) fsid: u64,
    /// `st_dev` of the layer root, used for same-filesystem comparisons.
    pub(in overlayfs) container_dev_id: DeviceId,
}

impl Layer {
    /// Builds the upper-layer (index 0) real object for `child_path`.
    pub(in overlayfs) fn child_real_object(&self, child_path: &Path) -> RealObject {
        RealObject::from_layer_path(
            0,
            RealPath::from_path(child_path),
            self.fsid,
            self.container_dev_id,
        )
    }

    /// Resolves `raw_path` into pinned layer-root parts, downgrading the
    /// `Path` into the layer-root anchor [`RealPath`].
    fn resolve_parts(raw_path: &str) -> Result<LayerParts> {
        // Missing paths surface the resolver's `ENOENT`; non-directory roots
        // fail with `ENOTDIR`.
        let path = resolve_root_path(raw_path)?;
        if !path.type_().is_directory() {
            return_errno_with_message!(Errno::ENOTDIR, "the layer root is not a directory");
        }
        Ok((
            RealPath::from_path(&path),
            path.fs(),
            path.metadata()?.container_dev_id,
        ))
    }
}
impl LayerStack {
    /// Rejects an overlap between `new` and every already-assembled layer root.
    ///
    /// - Same directory: identical dentry or inode objects.
    /// - Ancestor/descendant: one root lies within the other's hierarchy.
    /// - Mount boundary: parent chains never cross a mount root.
    ///
    /// Only layer roots are compared, so legal nested subdirectories are never rejected;
    /// violations return `EINVAL`.
    fn validate_layer_overlap(new: &Layer, others: &[&Layer]) -> Result<()> {
        let new_path = new.root_path.upgrade()?;
        let new_dentry = new_path.dentry();
        for other in others {
            let other_path = other.root_path.upgrade()?;
            let other_dentry = other_path.dentry();
            if Arc::ptr_eq(new_dentry, other_dentry)
                || Arc::ptr_eq(new.root_path.inode(), other.root_path.inode())
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

    /// Assembles the resolved upper/lower layer stack of an overlay mount.
    ///
    /// The upper root fails with `EROFS` when its backend is read-only and
    /// the overlay itself was not forced read-only. Non-empty `lower_dirs` is
    /// enforced here: an empty lower stack is rejected with `EINVAL`.
    pub(super) fn assemble(
        upper_dir: Option<String>,
        lower_dirs: Vec<String>,
        is_forced_read_only: bool,
    ) -> Result<Self> {
        let mut upper_parts = None;
        if let Some(raw_path) = upper_dir {
            let (root_path, fs, container_dev_id) = Layer::resolve_parts(&raw_path)?;
            if !is_forced_read_only && fs.flags().contains(FsFlags::RDONLY) {
                return_errno_with_message!(Errno::EROFS, "the upper filesystem is read-only");
            }
            upper_parts = Some((root_path, fs, container_dev_id));
        }

        if lower_dirs.is_empty() {
            return_errno_with_message!(
                Errno::EINVAL,
                "at least one lower layer is required to assemble the layer stack"
            );
        }
        let lower_parts: Vec<LayerParts> = lower_dirs
            .iter()
            .map(|raw_path| Layer::resolve_parts(raw_path))
            .collect::<Result<_>>()?;

        // The upper filesystem owns `fsid` 0 on writable overlays.
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

        let upper = upper_parts.map(|(root_path, fs, container_dev_id)| {
            let fsid = fsid_of_fn(&fs);
            Layer {
                root_path,
                fs,
                fsid,
                container_dev_id,
            }
        });
        let lowers = lower_parts
            .into_iter()
            .map(|(root_path, fs, container_dev_id)| {
                let fsid = fsid_of_fn(&fs);
                Layer {
                    root_path,
                    fs,
                    fsid,
                    container_dev_id,
                }
            })
            .collect::<Vec<_>>();

        let all_layers: Vec<&Layer> = upper.iter().chain(lowers.iter()).collect();
        for (index, new_layer) in all_layers.iter().enumerate() {
            Self::validate_layer_overlap(new_layer, &all_layers[index + 1..])?;
        }

        Ok(Self { upper, lowers })
    }

    /// Returns the writable upper layer, or `EROFS` when the stack has none.
    pub(in overlayfs) fn upper_layer(&self) -> Result<&Layer> {
        self.upper.as_ref().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay mount has no upper layer")
        })
    }

    /// Returns the ordered lower layers.
    pub(in overlayfs) fn lower_layers(&self) -> &[Layer] {
        &self.lowers
    }

    /// Rejects a workdir root that is the same as, an ancestor of, or a
    /// descendant of any lower layer root.
    ///
    /// The workdir is not a layer, so [`LayerStack::validate_layer_overlap`]
    /// cannot cover it; a nested workdir would place the staging workspace
    /// inside the lower tree. Violations return `EINVAL`.
    pub(super) fn validate_workdir_against_lowers(&self, workdir_path: &Path) -> Result<()> {
        let workdir_dentry = workdir_path.dentry();
        for lower in &self.lowers {
            let lower_path = lower.root_path.upgrade()?;
            let lower_dentry = lower_path.dentry();
            if Arc::ptr_eq(lower_dentry, workdir_dentry)
                || Arc::ptr_eq(lower.root_path.inode(), workdir_path.inode())
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "workdir must be distinct from every lower layer root"
                );
            }
            if workdir_dentry.is_equal_or_descendant_of(lower_dentry)
                || lower_dentry.is_equal_or_descendant_of(workdir_dentry)
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "workdir must not be an ancestor or descendant of a lower layer root"
                );
            }
        }
        Ok(())
    }

    /// Converts a copy-up origin layer index to the configured lower index.
    ///
    /// `layer_index()` counts the upper as position 0, so when the stack has
    /// an upper the origin's own lower position is `layer_index - 1`; both
    /// out-of-range forms fail with `EINVAL`.
    pub(in overlayfs) fn lower_layer_root_ino_for_origin(&self, layer_index: usize) -> Result<u64> {
        let lower_index = if self.upper.is_some() {
            layer_index.checked_sub(1).ok_or_else(|| {
                Error::with_message(
                    Errno::EINVAL,
                    "the origin source does not identify a configured lower layer",
                )
            })?
        } else {
            layer_index
        };
        let lower_layer = self.lowers.get(lower_index).ok_or_else(|| {
            Error::with_message(
                Errno::EINVAL,
                "the origin source does not identify a configured lower layer",
            )
        })?;
        Ok(lower_layer.root_path.inode().ino())
    }

    /// Collects the construction-local layer identity inputs for
    /// [`IdentityPolicy::new`](crate::fs::fs_impls::overlayfs::projection::IdentityPolicy::new).
    ///
    /// Returns the per-published-layer [`LowerLayerIdentity`] list (upper
    /// first when present) with the upper's entry position. The exclusion is
    /// by position, not by value: an upper sharing an underlying filesystem
    /// with a lower must not also drop the lower's entry.
    pub(super) fn collect_layer_devs(&self) -> (Vec<LowerLayerIdentity>, Option<usize>) {
        let layer_capacity = self.lowers.len() + if self.upper.is_some() { 1 } else { 0 };
        let mut layer_devs: Vec<LowerLayerIdentity> = Vec::with_capacity(layer_capacity);
        let upper_layer_dev_index = if let Some(upper) = self.upper.as_ref() {
            let index = layer_devs.len();
            layer_devs.push(LowerLayerIdentity {
                fsid: upper.fsid,
                container_dev_id: upper.container_dev_id,
                lower_layer_root_ino: upper.root_path.inode().ino(),
            });
            Some(index)
        } else {
            None
        };
        for lower in &self.lowers {
            layer_devs.push(LowerLayerIdentity {
                fsid: lower.fsid,
                container_dev_id: lower.container_dev_id,
                lower_layer_root_ino: lower.root_path.inode().ino(),
            });
        }
        (layer_devs, upper_layer_dev_index)
    }
}
