// SPDX-License-Identifier: MPL-2.0

//! EROFS decompression entry points.

mod lz4;

pub(super) use lz4::{decompress, prepare_input};
