// SPDX-License-Identifier: MPL-2.0

//! This module introduces the xarray crate and provides relevant support and interfaces for `XArray`.
extern crate xarray as xarray_crate;

pub use xarray_crate::{Cursor, CursorMut, XArray, XMark};
