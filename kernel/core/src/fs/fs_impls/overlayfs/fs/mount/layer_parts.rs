// SPDX-License-Identifier: MPL-2.0

//! Layer stack assembly for the overlay filesystem.
//!
//! This module contains the mount-time half of the layer model: the roots the
//! options name, resolved and validated once into [`MountRoots`], and the
//! mount's read-only state, layer stack, and claimed upper/workdir pair each
//! answered from that value.
//!
//! Lower layers are read-only: the overlay never writes the lower layers.
//!
//! - External concurrent modification of the lower layers is unsupported:
//!   the dev/ino identity translation and the inode identity-reuse cache
//!   assume a stable layer stack, and an external lower writer can corrupt
//!   the visible merge.
//! - Overlap between the upper, the workdir, and the lower layers is
//!   rejected at the mount boundary.

use super::{
    super::{
        super::layer::{Layer, LayerStack, ensure_distinct_non_overlapping},
        OverlayFs,
    },
    inuse::{UpperWorkdirInuse, Uuid},
    options::{MountOptions, UuidMode},
};
use crate::{
    fs::vfs::{
        file_system::FsFlags,
        path::{AT_FDCWD, Dentry, EmptyPathStr, FsPath, Path, PerMountFlags},
    },
    prelude::*,
};

/// The mount's resolved roots, validated: the layer roots and the upper/workdir claim's input.
pub(super) struct MountRoots {
    /// The upper root; absent on a mount given no upper.
    upper: Option<Path>,
    /// The workdir root; present exactly when `upper` is.
    work: Option<Path>,
    /// Every lower root, topmost first.
    lowers: Vec<Path>,
}

impl MountRoots {
    /// Resolves and validates the mount's roots, and settles the read-only state they imply.
    pub(super) fn resolve(ctx: &Context, options: &MountOptions) -> Result<Self> {
        // The pair is all-or-nothing before `MountOptions` is usable, so this guard only keeps the
        // unreachable mixed shape from reaching the resolutions below.
        if options.upper_dir.is_some() != options.work_dir.is_some() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the `workdir` mount option is required if and only if `upperdir` is specified"
            );
        }

        // Resolve original roots in option order: upperdir -> lowerdir -> workdir.
        let upper = options
            .upper_dir
            .as_ref()
            .map(|upper_dir| resolve_root_dir(ctx, upper_dir, "the layer root is not a directory"))
            .transpose()?;
        let lowers = resolve_lower_roots(ctx, &options.lower_dirs)?;
        let work = options
            .work_dir
            .as_ref()
            .map(|work_dir| resolve_root_dir(ctx, work_dir, "workdir is not a directory"))
            .transpose()?;

        let roots = Self {
            upper,
            work,
            lowers,
        };

        if options.uuid_mode == Some(UuidMode::On) && roots.is_effective_read_only(options) {
            warn!(
                "option `uuid=on` is ineffective on a read-only overlay; the overlay uuid is not persisted"
            );
        }
        if roots.upper_is_read_only() && !options.is_forced_read_only {
            warn!("the upperdir is read-only; the mount is treated as read-only");
        }

        // An overlay cannot back another's upper: writes re-enter the backing overlay.
        if let Some(upper_root) = &roots.upper
            && upper_root
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

        let (Some(upper_root), Some(work_root)) = (&roots.upper, &roots.work) else {
            roots.validate_overlap()?;
            return Ok(roots);
        };

        UpperWorkdirInuse::validate_pair(upper_root, work_root)?;
        roots.validate_overlap()?;
        for lower in roots.lowers.iter().map(Path::dentry) {
            ensure_distinct_non_overlapping(work_root.dentry(), lower)?;
        }
        Ok(roots)
    }

    /// Claims the upper/workdir pair, absent on a mount with no upper.
    pub(super) fn claim_upper_workdir(
        &self,
        options: &MountOptions,
    ) -> Result<Option<UpperWorkdirInuse>> {
        let (Some(upper_root), Some(work_root)) = (&self.upper, &self.work) else {
            return Ok(None);
        };
        let uuid_mode = options.uuid_mode.unwrap_or(UuidMode::Auto);
        let uuid = if self.is_effective_read_only(options) {
            Ok(Uuid::generate())
        } else {
            UpperWorkdirInuse::determine_identity(
                upper_root.dentry(),
                uuid_mode,
                options.xattr_namespace,
            )
        }?;
        Ok(Some(UpperWorkdirInuse::claim(
            upper_root.dentry(),
            work_root.dentry(),
            uuid,
        )?))
    }

    /// Builds the layer stack from the resolved roots.
    pub(super) fn into_layer_stack(self) -> Result<LayerStack> {
        let upper = self.upper.as_ref().map(Layer::from_path).transpose()?;
        let lowers = self
            .lowers
            .iter()
            .map(Layer::from_path)
            .collect::<Result<Vec<_>>>()?;
        Ok(LayerStack::new(upper, lowers))
    }

    pub(super) fn is_effective_read_only(&self, options: &MountOptions) -> bool {
        options.is_forced_read_only || self.upper.is_none() || self.upper_is_read_only()
    }

    fn upper_is_read_only(&self) -> bool {
        self.upper.as_ref().is_some_and(upper_path_is_read_only)
    }

    /// Rejects any two roots being the same or nested.
    fn validate_overlap(&self) -> Result<()> {
        let mut roots: Vec<&Arc<Dentry>> = Vec::new();
        if let Some(upper) = &self.upper {
            roots.push(upper.dentry());
        }
        for lower in &self.lowers {
            roots.push(lower.dentry());
        }

        for index in 0..roots.len() {
            for other_index in (index + 1)..roots.len() {
                ensure_distinct_non_overlapping(roots[index], roots[other_index])?;
            }
        }
        Ok(())
    }
}

fn upper_path_is_read_only(upper: &Path) -> bool {
    upper.mount_node().fs().flags().contains(FsFlags::RDONLY)
        || upper.mount_node().flags().contains(PerMountFlags::RDONLY)
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
