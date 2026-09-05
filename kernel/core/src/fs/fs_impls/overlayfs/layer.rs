// SPDX-License-Identifier: MPL-2.0

//! The layer-model types of an overlay mount.
//!
//! Mount-time assembly and validation of these types lives in
//! `fs/mount/layer_parts.rs`.

#![short_vis_path::add(overlayfs)]

use device_id::DeviceId;

use super::real::RealObject;
use crate::{
    fs::vfs::path::{Dentry, Mount},
    prelude::*,
};

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

/// The durable identity of one layer: its backing device and the pinned root's inode number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) struct LayerIdentity {
    pub(in overlayfs) container_dev_id: DeviceId,
    pub(in overlayfs) root_ino: u64,
}

/// One pinned real directory root of the mount: the upper or a read-only lower.
#[derive(Debug)]
pub(super) struct Layer {
    pub(super) mount: Arc<Mount>,
    pub(super) fsid: u64,
    pub(super) container_dev_id: DeviceId,
}

impl Layer {
    pub(super) fn root_dentry(&self) -> &Arc<Dentry> {
        self.mount.root_dentry()
    }

    /// Derives the durable identity pair from the pinned layer root.
    pub(super) fn identity(&self) -> LayerIdentity {
        LayerIdentity {
            container_dev_id: self.container_dev_id,
            root_ino: self.root_dentry().inode().ino(),
        }
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

    /// Resolves a durable identity pair back to its mount-local lower layer, else `None`.
    pub(super) fn resolve_lower_layer(&self, identity: &LayerIdentity) -> Option<&Layer> {
        let mut matched: Option<&Layer> = None;
        for layer in &self.lowers {
            if layer.identity() == *identity {
                match matched {
                    None => matched = Some(layer),
                    Some(existing) if existing.fsid == layer.fsid => {}
                    Some(_) => return None,
                }
            }
        }
        matched
    }

    /// Whether every layer is rooted on one underlying filesystem; the empty set reports `true`.
    pub(super) fn is_all_layers_same_fs(&self) -> bool {
        let mut devs = self
            .upper
            .iter()
            .chain(self.lowers.iter())
            .map(|layer| layer.container_dev_id);
        match devs.next() {
            Some(first) => devs.all(|dev| dev == first),
            None => true,
        }
    }
}

/// The ordered real objects behind one logical overlay object, and which supplies its metadata.
#[derive(Clone, Debug)]
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

    pub(super) fn is_merged(&self) -> bool {
        (self.upper.is_some() && !self.lowers.is_empty()) || self.lowers.len() > 1
    }

    pub(super) fn visible_source(&self) -> &RealObject {
        match &self.upper {
            Some(upper) => upper,
            None => &self.lowers[0],
        }
    }
}
