// SPDX-License-Identifier: MPL-2.0

//! The overlay filesystem: one or more read-only lower directories with an optional writable upper
//! directory on top of them, merged into a single directory tree.
//!
//! # Module map
//!
//! | Submodule | Responsibility |
//! |---|---|
//! | [`fs`] | [`OverlayFs`](fs::OverlayFs), the per-mount filesystem object, its VFS-facing superblock surface, and the `overlay` registration |
//! | [`inode`] | [`OverlayInode`](inode::OverlayInode), the logical object the VFS sees, with the dev/ino identity system and the namespace mutations |
//! | [`layer`] | [`Layer`](layer::Layer) and [`LayerStack`](layer::LayerStack), the layer-model types |
//! | [`real`] | [`RealObject`](real::RealObject) and [`RealObjectStack`](real::RealObjectStack), the references to the underlying objects |

mod fs;
mod inode;
mod layer;
mod real;

pub(super) use self::fs::init;
