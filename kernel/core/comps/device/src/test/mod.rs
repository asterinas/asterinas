// SPDX-License-Identifier: MPL-2.0

//! Device-model tests, grouped by the behavior they verify.
//!
//! - `registry`: initialization.
//! - `registration`: device registration.
//! - `attributes`: device attributes.
//! - `observers`: class observers.
//! - `names`: device name validation.
//!
//! `toy` and `utils` provide shared test helpers.

mod attributes;
mod names;
mod observers;
mod registration;
mod registry;
mod toy;
mod utils;
