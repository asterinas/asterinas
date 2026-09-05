// SPDX-License-Identifier: MPL-2.0

//! Layer stack assembly for the overlay filesystem.
//!
//! This module contains the mount-time half of the layer model: root
//! resolution, resolve/validate/assemble phased assembly, and the one
//! assembly entry [`LayerStack::assemble`] that every later mount step runs
//! through. Assembly owns the resolved [`Path`]s: it resolves the original
//! roots, validates them, derives the mount's read-only state, warns when the
//! upper itself is read-only, and hands back the layer stack with the claimed
//! upper/workdir pair.
//!
//! Lower layers are read-only: the overlay never writes the lower layers.
//!
//! - External concurrent modification of the lower layers is unsupported:
//!   the dev/ino identity translation and the inode identity-reuse cache
//!   assume a stable layer stack, and an external lower writer can corrupt
//!   the visible merge.
//! - Overlap between the upper, the workdir, and the lower layers is
//!   rejected at the mount boundary.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v7.0/source/Documentation/filesystems/overlayfs.rst#L350-L364>
//!   (Linux overlayfs parity; stacks colon-separated lowerdirs with the first entry topmost)
//! - <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/super.c#L1273>
//!   (Linux `ovl_check_overlapping_layers`)
//! - <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/ovl_entry.h#L33-L42>
//!   (Linux `ovl_layer[].fsid`, upper fsid 0)

use super::{
    super::{
        super::layer::{Layer, LayerStack, ensure_distinct_non_overlapping},
        OverlayFs,
        policy::UuidMode,
    },
    inuse::{UpperWorkdirInuse, Uuid},
    options::MountOptions,
};
use crate::{
    fs::vfs::{
        file_system::FsFlags,
        path::{AT_FDCWD, Dentry, EmptyPathStr, FsPath, Path, PerMountFlags},
    },
    prelude::*,
};

impl LayerStack {
    /// Resolves and validates the mount's roots, derives the mount's read-only
    /// state, warns when the upper itself is read-only, and assembles the layer
    /// stack and the upper/workdir claim.
    ///
    /// Returns the stack, whether the mount is effectively read-only, and the
    /// claimed pair when the mount has an upper.
    pub(super) fn assemble(
        ctx: &Context,
        options: &MountOptions,
    ) -> Result<(Self, bool, Option<UpperWorkdirInuse>)> {
        // Resolve original roots in option order: upperdir -> lowerdir -> workdir.
        let (upper_path, work_path, lower_paths) = match (&options.upper_dir, &options.work_dir) {
            (Some(upper_dir), Some(work_dir)) => {
                let upper_path =
                    resolve_root_dir(ctx, upper_dir, "the layer root is not a directory")?;
                let lower_paths = resolve_lower_roots(ctx, &options.lower_dirs)?;
                let work_path = resolve_root_dir(ctx, work_dir, "workdir is not a directory")?;
                (Some(upper_path), Some(work_path), lower_paths)
            }
            (None, None) => (None, None, resolve_lower_roots(ctx, &options.lower_dirs)?),
            // The pair is all-or-nothing before `MountOptions` is usable, so a
            // mixed pair cannot arrive here; the arm keeps the match total.
            (Some(_), None) | (None, Some(_)) => {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the `workdir` mount option is required if and only if `upperdir` is specified"
                );
            }
        };

        let is_upper_read_only = upper_path.as_ref().is_some_and(|upper| {
            upper.mount_node().fs().flags().contains(FsFlags::RDONLY)
                || upper.mount_node().flags().contains(PerMountFlags::RDONLY)
        });
        let is_effective_read_only =
            options.is_forced_read_only || upper_path.is_none() || is_upper_read_only;
        if options.uuid_mode == Some(UuidMode::On) && is_effective_read_only {
            info!(
                "option `uuid=on` is ineffective on a read-only overlay; the overlay uuid is not persisted"
            );
        }
        if is_upper_read_only && !options.is_forced_read_only {
            warn!(
                "the overlay upperdir is read-only; the overlay is treated as read-only \
                 (`uuid=on` is ineffective, no workdir is prepared, and upper capabilities are not probed)"
            );
        }

        let xattr_namespace = options.xattr_namespace();
        let lower_dentries: Vec<&Arc<Dentry>> = lower_paths.iter().map(Path::dentry).collect();

        let (Some(upper_root), Some(work_root)) = (&upper_path, &work_path) else {
            Self::validate_layer_overlap(None, &lower_dentries)?;
            let layer_stack = Self::build(None, &lower_paths)?;
            return Ok((layer_stack, is_effective_read_only, None));
        };

        // An overlay cannot back another's upper: writes re-enter the backing overlay.
        if upper_root
            .mount_node()
            .fs()
            .downcast_ref::<OverlayFs>()
            .is_some()
        {
            return Err(Error::with_message(
                Errno::EINVAL,
                "the overlay upperdir must not be on an overlayfs",
            ));
        }

        UpperWorkdirInuse::validate_pair(upper_root, work_root)?;
        Self::validate_layer_overlap(Some(upper_root.dentry()), &lower_dentries)?;
        for lower in &lower_dentries {
            ensure_distinct_non_overlapping(
                work_root.dentry(),
                lower,
                "workdir must be distinct from every lower layer root",
                "workdir must not be an ancestor or descendant of a lower layer root",
            )?;
        }

        let uuid_mode = options.uuid_mode.unwrap_or(UuidMode::Auto);
        let uuid = if is_effective_read_only {
            Ok(Uuid::generate())
        } else {
            UpperWorkdirInuse::determine_identity(upper_root.dentry(), uuid_mode, xattr_namespace)
        }?;

        let inuse_pair = UpperWorkdirInuse::claim(upper_root.dentry(), work_root.dentry(), uuid)?;
        let layer_stack = Self::build(Some(upper_root), &lower_paths)?;
        Ok((layer_stack, is_effective_read_only, Some(inuse_pair)))
    }

    /// Rejects any two layer roots being the same or nested.
    fn validate_layer_overlap(upper: Option<&Arc<Dentry>>, lowers: &[&Arc<Dentry>]) -> Result<()> {
        let mut roots: Vec<&Arc<Dentry>> = Vec::new();
        if let Some(upper) = upper {
            roots.push(upper);
        }
        for lower in lowers {
            roots.push(lower);
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

    /// Builds the stack from the resolved roots.
    fn build(upper: Option<&Path>, lowers: &[Path]) -> Result<Self> {
        let upper = upper.map(Layer::from_path).transpose()?;
        let lowers = lowers
            .iter()
            .map(Layer::from_path)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { upper, lowers })
    }
}

impl Layer {
    /// Builds one layer from a resolved root path.
    fn from_path(path: &Path) -> Result<Self> {
        let container_dev_id = path.metadata()?.container_dev_id;
        let root_dentry = path.dentry().clone();
        let fs = path.mount_node().fs().clone();
        Ok(Self {
            root_dentry,
            fs,
            container_dev_id,
        })
    }
}

/// Resolves one layer root path and rejects it when it is not a directory.
fn resolve_root_dir(ctx: &Context, raw_path: &str, not_dir_msg: &'static str) -> Result<Path> {
    let fs_path = FsPath::from_fd_at(AT_FDCWD, raw_path, EmptyPathStr::Reject)?;
    let path = ctx
        .thread_local
        .borrow_fs()
        .resolver()
        .read()
        .lookup_no_follow(&fs_path)?;
    if !path.type_().is_directory() {
        return_errno_with_message!(Errno::ENOTDIR, not_dir_msg);
    }
    Ok(path)
}

/// Resolves every lower layer root.
fn resolve_lower_roots(ctx: &Context, raw_paths: &[String]) -> Result<Vec<Path>> {
    raw_paths
        .iter()
        .map(|raw_path| resolve_root_dir(ctx, raw_path, "the layer root is not a directory"))
        .collect()
}
