// SPDX-License-Identifier: MPL-2.0

//! The layer-model types of an overlay mount.
//!
//! A layer is built from its resolved root by [`Layer::from_path`]; the assembly and validation
//! of the whole stack stays in `fs/mount/layer_parts.rs`.

use device_id::DeviceId;

use crate::{
    fs::vfs::{
        file_system::FileSystem,
        path::{Dentry, Path},
    },
    prelude::*,
};

/// One pinned real directory root of the mount: the upper or a read-only lower.
#[derive(Debug)]
pub(super) struct Layer {
    root_dentry: Arc<Dentry>,
    fs: Arc<dyn FileSystem>,
    container_dev_id: DeviceId,
}

impl Layer {
    /// Builds one layer from a resolved root path.
    pub(super) fn from_path(path: &Path) -> Result<Self> {
        let container_dev_id = path.metadata()?.container_dev_id;
        let root_dentry = path.dentry().clone();
        let fs = path.mount_node().fs().clone();
        Ok(Self {
            root_dentry,
            fs,
            container_dev_id,
        })
    }

    pub(super) fn root_dentry(&self) -> &Arc<Dentry> {
        &self.root_dentry
    }

    pub(super) fn fs(&self) -> &Arc<dyn FileSystem> {
        &self.fs
    }

    pub(super) fn container_dev_id(&self) -> DeviceId {
        self.container_dev_id
    }
}

/// The ordered, mount-fixed layer collection: upper first, then lowers topmost-first.
#[derive(Debug)]
pub(super) struct LayerStack {
    upper: Option<Layer>,
    lowers: Vec<Layer>,
}

impl LayerStack {
    /// Collects the mount's validated upper and lowers into the stack the mount presents.
    pub(super) fn new(upper: Option<Layer>, lowers: Vec<Layer>) -> Self {
        Self { upper, lowers }
    }

    pub(super) fn upper_layer(&self) -> Result<&Layer> {
        self.upper.as_ref().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay mount has no upper layer")
        })
    }

    pub(super) fn lower_layers(&self) -> &[Layer] {
        &self.lowers
    }
}

pub(super) fn ensure_distinct_non_overlapping(a: &Arc<Dentry>, b: &Arc<Dentry>) -> Result<()> {
    if Arc::ptr_eq(a.inode(), b.inode())
        || a.is_equal_or_descendant_of(b)
        || b.is_equal_or_descendant_of(a)
    {
        return_errno_with_message!(
            Errno::EINVAL,
            "overlay roots must be distinct and must not be nested"
        );
    }
    Ok(())
}
