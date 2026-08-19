// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Mount build-time subtree: options, layer assembly, claims, and policy.
//!
//! This module contains only mount construction state. The per-mount
//! overlay filesystem object lives in the `superblock` module and
//! VFS registration lives in the top-level `fs_type` module.

mod build;
pub(super) mod claims;
pub(super) mod layers;
mod options;
pub(super) mod policy;

pub(super) use layers::RealPath;
pub(super) use options::XinoMode;
