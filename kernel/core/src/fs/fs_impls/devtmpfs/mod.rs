// SPDX-License-Identifier: MPL-2.0

//! The device temporary filesystem.
//!
//! This module implements `devtmpfs` as a singleton filesystem backed by
//! `tmpfs`. Device subsystems submit node-management requests through
//! [`create_node`] and [`delete_node`]; the requests are serialized and handled
//! by the dedicated kernel thread `devtmpfsd`. This gives node operations a
//! single VFS execution context instead of mutating the filesystem tree directly
//! from arbitrary device-registration contexts, and keeps these operations
//! independent of the credentials of the device-registration caller, following
//! Linux's devtmpfs design.
//!
//! Mounting policy follows the selected init path. If an initramfs init is
//! selected, either by `rdinit=` or by the default `/init` lookup, the kernel
//! does not mount devtmpfs automatically; the selected initramfs init program is
//! responsible for mounting it if needed. Otherwise, Asterinas boots from the
//! configured root filesystem and the kernel mounts this singleton on `/dev`
//! during first-process initialization.

mod fs;
mod tree;
mod worker;

pub(in crate::fs) use fs::singleton;
pub(crate) use tree::{DevtmpfsNode, DevtmpfsNodeMeta};
pub(crate) use worker::{create_node, delete_node};

pub(super) fn init() {
    fs::init();
}

pub(super) fn init_in_first_kthread() {
    worker::init();
}
