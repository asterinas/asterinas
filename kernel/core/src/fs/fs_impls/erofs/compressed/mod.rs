// SPDX-License-Identifier: MPL-2.0

//! Shared compressed-input ownership and logical output-page tasks.
//!
//! Mapping, pcluster coordination, and output completion have separate owners.
//! The inode backend maps output pages and registers their fragments here.

pub(super) use pcluster::{
    MAX_INPUT_RANGE_PAGES, MAX_NEIGHBOR_INPUTS, Pcluster, PclusterJob, PclusterManager,
};

mod batch_wait;
pub(super) mod mapping;
pub(super) mod output;
mod pcluster;
