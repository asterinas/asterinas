// SPDX-License-Identifier: MPL-2.0

//! The real-object reference model beneath the overlay namespace.
//!
//! Anchor validity follows the overlay lifetime: the owning layer strongly
//! holds its private clone view, so a reachable logical object never
//! observes a dead anchor.

#![short_vis_path::add(overlayfs)]

use crate::{
    fs::vfs::{inode::Inode, path::Dentry},
    prelude::*,
};

/// One underlying filesystem entry as seen from one layer: its layer index and dentry.
#[derive(Clone, Debug)]
pub(super) struct RealObject {
    layer_index: usize,
    dentry: Arc<Dentry>,
}

impl RealObject {
    pub(super) fn new(layer_index: usize, dentry: Arc<Dentry>) -> Self {
        Self {
            layer_index,
            dentry,
        }
    }

    pub(super) fn layer_index(&self) -> usize {
        self.layer_index
    }

    pub(super) fn dentry(&self) -> &Arc<Dentry> {
        &self.dentry
    }

    pub(super) fn real_inode(&self) -> &Arc<dyn Inode> {
        self.dentry.inode()
    }
}

/// The visible source's layer fsid paired with its real inode number.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct RealObjectKey {
    fsid: u64,
    real_ino: u64,
}

impl RealObjectKey {
    pub(super) fn from_source(fsid: u64, real: &RealObject) -> Self {
        Self {
            fsid,
            real_ino: real.real_inode().ino(),
        }
    }
}
