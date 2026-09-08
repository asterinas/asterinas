// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Layer stack assembly for the overlay filesystem.
//!
//! This module contains the mount-time half of the layer model: root path
//! resolution, instance-stability probing, two-phase `LayerParts`
//! assembly, and [`LayerStack::assemble`]. The [`Layer`] / [`LayerStack`]
//! types are defined by the layer model itself; only the construction
//! logic that runs during `OverlayFs::new` lives here.
//!
//! Every layer (and, riding the upper view, the workdir) is assembled with a
//! private unregistered clone view rooted at its resolved path, reusing the
//! existing VFS mount-clone primitive.
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

use device_id::DeviceId;

use super::super::super::layer::{Layer, LayerStack};
use crate::{
    fs::vfs::{
        file_system::{FileSystem, FsFlags},
        inode::Inode,
        path::{AT_FDCWD, EmptyPathStr, FsPath, Mount, Path},
    },
    prelude::*,
};

type LayerParts = (Arc<Mount>, DeviceId);

pub(super) fn resolve_root_path(raw_path: &str) -> Result<Path> {
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

impl Layer {
    fn resolve_parts(raw_path: &str) -> Result<LayerParts> {
        let path = resolve_root_path(raw_path)?;
        if !path.type_().is_directory() {
            return_errno_with_message!(Errno::ENOTDIR, "the layer root is not a directory");
        }
        let container_dev_id = path.metadata()?.container_dev_id;
        let view = path.mount_node().clone_mount(path.dentry(), &Weak::new())?;
        Ok((view, container_dev_id))
    }
}

impl LayerStack {
    pub(super) fn assemble(
        upper_dir: Option<String>,
        lower_dirs: Vec<String>,
        is_forced_read_only: bool,
    ) -> Result<Self> {
        let mut upper_parts = None;
        if let Some(raw_path) = upper_dir {
            let (mount, container_dev_id) = Layer::resolve_parts(&raw_path)?;
            if !is_forced_read_only && mount.fs().flags().contains(FsFlags::RDONLY) {
                return_errno_with_message!(Errno::EROFS, "the upper filesystem is read-only");
            }
            upper_parts = Some((mount, container_dev_id));
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

        let upper = upper_parts.map(|(mount, container_dev_id)| {
            let fsid = fsid_of_fn(mount.fs());
            Layer {
                mount,
                fsid,
                container_dev_id,
            }
        });
        let lowers = lower_parts
            .into_iter()
            .map(|(mount, container_dev_id)| {
                let fsid = fsid_of_fn(mount.fs());
                Layer {
                    mount,
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
}
