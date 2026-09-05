// SPDX-License-Identifier: MPL-2.0

//! The layer-model types of an overlay mount.
//!
//! Mount-time assembly and validation of these types lives in
//! `fs/mount/layer_parts.rs`.

#![short_vis_path::add(overlayfs)]

use device_id::DeviceId;

use super::real::RealObject;
use crate::{
    fs::vfs::{file_system::FileSystem, path::Dentry},
    prelude::*,
};

/// One pinned real directory root of the mount: the upper or a read-only lower.
#[derive(Debug)]
pub(super) struct Layer {
    pub(super) root_dentry: Arc<Dentry>,
    pub(super) fs: Arc<dyn FileSystem>,
    pub(super) container_dev_id: DeviceId,
}

impl Layer {
    pub(super) fn root_dentry(&self) -> &Arc<Dentry> {
        &self.root_dentry
    }
}

/// The ordered, mount-fixed layer collection: upper first, then lowers topmost-first.
#[derive(Debug)]
pub(super) struct LayerStack {
    pub(super) upper: Option<Layer>,
    pub(super) lowers: Vec<Layer>,
}

impl LayerStack {
    pub(super) fn upper_layer(&self) -> Result<&Layer> {
        self.upper.as_ref().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay mount has no upper layer")
        })
    }

    pub(super) fn lower_layers(&self) -> &[Layer] {
        &self.lowers
    }
}

/// The ordered real objects behind one logical overlay object, and which supplies its metadata.
#[derive(Debug)]
pub(super) struct RealObjectStack {
    pub(super) upper: Option<RealObject>,
    pub(super) lowers: Vec<RealObject>,
}

impl RealObjectStack {
    pub(super) fn new(upper: Option<RealObject>, lowers: Vec<RealObject>) -> Self {
        debug_assert!(upper.is_some() || !lowers.is_empty());
        Self { upper, lowers }
    }

    pub(super) fn upper_only(upper: RealObject) -> Self {
        Self {
            upper: Some(upper),
            lowers: Vec::new(),
        }
    }

    pub(super) fn lower_only(lower: RealObject) -> Self {
        Self {
            upper: None,
            lowers: vec![lower],
        }
    }

    pub(super) fn visible_source(&self) -> &RealObject {
        match &self.upper {
            Some(upper) => upper,
            None => &self.lowers[0],
        }
    }
}

pub(in overlayfs) fn ensure_distinct_non_overlapping(
    a: &Arc<Dentry>,
    b: &Arc<Dentry>,
    distinct_msg: &'static str,
    overlap_msg: &'static str,
) -> Result<()> {
    if Arc::ptr_eq(a.inode(), b.inode()) {
        return_errno_with_message!(Errno::EINVAL, distinct_msg);
    }
    if a.is_equal_or_descendant_of(b) || b.is_equal_or_descendant_of(a) {
        return_errno_with_message!(Errno::EINVAL, overlap_msg);
    }
    Ok(())
}
