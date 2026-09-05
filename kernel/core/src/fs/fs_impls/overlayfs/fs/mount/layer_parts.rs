// SPDX-License-Identifier: MPL-2.0

//! Layer stack assembly for the overlay filesystem.
//!
//! This module contains the mount-time half of the layer model: root path
//! resolution, instance-stability probing, resolve/validate/build phased
//! assembly, and the private clone views that back [`Layer`] / [`LayerStack`].
//!
//! Every layer (and, riding the upper view, the workdir) starts as an original
//! resolved [`Path`]. Validation runs against those original paths before any
//! private clone view is built; [`build_layer_stack`] then constructs the
//! private views and assigns layer filesystem IDs.
//!
//! Lower layers are read-only: the overlay never writes the lower layers.
//!
//! - Non-`default_permissions` mounts promote mutating paths to the upper
//!   first.
//! - `default_permissions` mounts have a known limitation: the persisted
//!   directory-merging staleness marker (the impure xattr record under the
//!   mount's selected private prefix) is not refreshed after mutations, so the
//!   marker can remain stale. The limitation is scoped to that persisted
//!   marker; the other layer-stack invariants in this module still hold.
//! - External concurrent modification of the lower layers is unsupported:
//!   the dev/ino identity translation and the inode identity-reuse cache
//!   assume a stable layer stack, and an external lower writer can corrupt
//!   the visible merge.
//! - Overlap between the upper, the workdir, and the lower layers is
//!   rejected at the mount boundary — the one corruption form detectable
//!   at mount time; read-write lower backends remain accepted.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v7.0/source/Documentation/filesystems/overlayfs.rst#L350-L364>
//!   (Linux overlayfs parity; stacks colon-separated lowerdirs with the first entry topmost)
//! - <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/super.c#L1273>
//!   (Linux `ovl_check_overlapping_layers`)
//! - <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/ovl_entry.h#L33-L42>
//!   (Linux `ovl_layer[].fsid`, upper fsid 0)

#![short_vis_path::add(overlayfs)]

use super::super::super::layer::{Layer, LayerStack, ensure_distinct_non_overlapping};
use crate::{
    fs::vfs::{
        file_system::FileSystem,
        inode::Inode,
        path::{AT_FDCWD, Dentry, EmptyPathStr, FsPath, Path},
    },
    prelude::*,
};

fn resolve_root_path(raw_path: &str) -> Result<Path> {
    let fs_path = FsPath::from_fd_at(AT_FDCWD, raw_path, EmptyPathStr::Reject)?;
    super::super::super::with_current_posix_thread(|_task, posix_thread| {
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

pub(super) fn resolve_root_dir(raw_path: &str, not_dir_msg: &'static str) -> Result<Path> {
    let path = resolve_root_path(raw_path)?;
    if !path.type_().is_directory() {
        return_errno_with_message!(Errno::ENOTDIR, not_dir_msg);
    }
    Ok(path)
}

/// Verifies the backend returns the same pinned inode instance for `raw_path`.
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

pub(super) fn resolve_lower_roots(raw_paths: &[String]) -> Result<Vec<Path>> {
    raw_paths
        .iter()
        .map(|raw_path| resolve_root_dir(raw_path, "the layer root is not a directory"))
        .collect()
}

pub(super) fn validate_layer_overlap(upper: Option<&Path>, lowers: &[Path]) -> Result<()> {
    let mut roots: Vec<&Arc<Dentry>> = Vec::new();
    if let Some(upper) = upper {
        roots.push(upper.dentry());
    }
    for lower in lowers {
        roots.push(lower.dentry());
    }

    for index in 0..roots.len() {
        for other_index in (index + 1)..roots.len() {
            ensure_distinct_non_overlapping(
                roots[index],
                roots[other_index],
                "overlay layer roots must be distinct directories",
                "overlay layer roots must not be each other's ancestor or descendant",
            )?;
        }
    }
    Ok(())
}

pub(super) fn validate_workdir_against_lowers(workdir: &Path, lowers: &[Path]) -> Result<()> {
    for lower in lowers {
        ensure_distinct_non_overlapping(
            workdir.dentry(),
            lower.dentry(),
            "workdir must be distinct from every lower layer root",
            "workdir must not be an ancestor or descendant of a lower layer root",
        )?;
    }
    Ok(())
}

pub(super) fn build_layer_stack(upper: Option<&Path>, lowers: &[Path]) -> Result<LayerStack> {
    let mut unique_fses: Vec<Arc<dyn FileSystem>> = Vec::new();
    let upper = upper
        .map(|path| layer_from_path(path, &mut unique_fses))
        .transpose()?;
    let lowers = lowers
        .iter()
        .map(|path| layer_from_path(path, &mut unique_fses))
        .collect::<Result<Vec<_>>>()?;
    Ok(LayerStack { upper, lowers })
}

fn layer_from_path(path: &Path, unique_fses: &mut Vec<Arc<dyn FileSystem>>) -> Result<Layer> {
    let container_dev_id = path.metadata()?.container_dev_id;
    let mount = path.mount_node().clone_detached(path.dentry())?;
    let fs = mount.fs().clone();
    let fsid = if let Some(index) = unique_fses
        .iter()
        .position(|seen_fs| Arc::ptr_eq(seen_fs, &fs))
    {
        index as u64
    } else {
        unique_fses.push(fs);
        (unique_fses.len() - 1) as u64
    };
    Ok(Layer {
        mount,
        fsid,
        container_dev_id,
    })
}
