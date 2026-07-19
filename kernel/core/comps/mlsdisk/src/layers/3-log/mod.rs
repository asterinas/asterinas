// SPDX-License-Identifier: MPL-2.0

//! The layer of transactional logging.
//!
//! `TxLogStore` is a transactional, log-oriented file system.
//! It supports creating, deleting, listing, reading, and writing `TxLog`s.
//! Each `TxLog` is an append-only log, and assigned an unique `TxLogId`.
//! All `TxLogStore`'s APIs should be called within transactions (`TX`).

mod chunk;
mod raw_log;
mod tx_log;

pub use self::tx_log::{TxLog, TxLogId, TxLogStore};
