// SPDX-License-Identifier: MPL-2.0

//! The layer of secure virtual disk.
//!
//! `SwornDisk` provides three block I/O interfaces, `read()`, `write()` and `sync()`.
//! `SwornDisk` protects a logical block of user data using authenticated encryption.
//! The metadata of the encrypted logical blocks are inserted into a secure index `TxLsmTree`.
//!
//! `SwornDisk`'s backed untrusted host disk space is managed in `BlockAlloc`. Block reclamation can be
//! delayed to user-defined callbacks on `TxLsmTree`.
//! `SwornDisk` supports buffering written logical blocks.
//!
//! # Usage Example
//!
//! Write, sync then read blocks from `SwornDisk`.
//!
//! ```
//! let nblocks = 1024;
//! let mem_disk = MemDisk::create(nblocks)?;
//! let root_key = Key::random();
//! let sworndisk = SwornDisk::create(mem_disk.clone(), root_key)?;
//!
//! let num_rw = 128;
//! let mut rw_buf = Buf::alloc(1)?;
//! for i in 0..num_rw {
//!     rw_buf.as_mut_slice().fill(i as u8);
//!     sworndisk.write(i as Lba, rw_buf.as_ref())?;
//! }
//! sworndisk.sync()?;
//! for i in 0..num_rw {
//!     sworndisk.read(i as Lba, rw_buf.as_mut())?;
//!     assert_eq!(rw_buf.as_slice()[0], i as u8);
//! }
//! ```

mod bio;
mod block_alloc;
mod data_buf;
mod sworndisk;

pub use self::sworndisk::SwornDisk;
