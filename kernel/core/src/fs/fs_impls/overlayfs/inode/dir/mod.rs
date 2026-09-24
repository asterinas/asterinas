// SPDX-License-Identifier: MPL-2.0

//! The overlayfs namespace-mutation and whiteout subsystem.
//!
//! This module hosts the four namespace-mutation recipes as its submodules —
//! create, link, remove, and rename — with the whiteout mechanics their
//! publications go through. Each recipe file owns its VFS entry.
//!
//! Key concepts:
//! - **entry sequence**: the removal and rename recipes decide the refusals that need no promotion
//!   before they promote; every recipe then promotes the objects its mutation writes through — the
//!   read-only gate and the copy-up promotion — and takes the parent lock only after that, with the
//!   promotion work already done by then; the promotion set differs per recipe.
//! - **parent directory transaction**: the lock every mutation entry takes for the parent it
//!   changes, and which the removal and rename recipes then extend to the object whose name they
//!   remove or displace, so every lock edge stays parent-to-descendant.
//!
//! # Module map
//!
//! | Submodule | Responsibility |
//! |---|---|
//! | [`create`] | the create-object dispatch for the three create-family entries |
//! | [`link`] | the hard-link recipe and the target classification it decides |
//! | [`remove`] | the shared unlink/rmdir recipe and the action it decides |
//! | [`rename`] | the rename recipe and the whiteout action it picks |
//! | [`whiteout`] | the whiteout cache and its publish mechanics |

pub(super) mod whiteout;

mod create;
mod link;
mod remove;
mod rename;
