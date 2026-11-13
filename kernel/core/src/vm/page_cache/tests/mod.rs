// SPDX-License-Identifier: MPL-2.0

//! Tests for buffered I/O and page-cache commit, resize, flush, and eviction.
//!
//! See `README.md` for coverage boundaries and the guide to adding tests.

mod base;
mod buffered_io;
mod commit;
mod evict;
mod flush;
mod resize;
