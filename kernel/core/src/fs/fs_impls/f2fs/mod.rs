// SPDX-License-Identifier: MPL-2.0

//! F2FS (Flash-Friendly File System) implementation.
//!
//! This module implements an F2FS driver that is compatible with volumes
//! created by `mkfs.f2fs`. After [`init`] registers the filesystem type, the
//! VFS can mount F2FS volumes with `mount -t f2fs <device> <mountpoint>`.
//!
//! Reads and basic writes are supported. Files can be created, written,
//! appended to, truncated, deleted, and renamed; directories can be created,
//! nested, removed, and renamed (including across directories); symbolic
//! links can be created and read. Writes are applied in place: free blocks
//! are taken from the SIT, and the NAT, SIT, and SSA areas are updated
//! immediately, so the data survives a remount.
//!
//! # Supported on-disk features
//!
//! - 4 KiB blocks and segments of 512 blocks, as produced by `mkfs.f2fs`
//!   on x86-64.
//! - The two checkpoint packs, including CRC-independent parsing and the
//!   NAT journal stored in node summary blocks.
//! - The NAT area, direct node blocks, two indirect node blocks, and the
//!   double-indirect node.
//! - Inline data for small files and inline dentries for small directories,
//!   including the per-inode inline xattr reservation.
//! - Fast and block-backed symbolic links.
//!
//! Volumes using encryption, compression, fs-verity, casefold, zoned block
//! devices, atomic writes, or multiple devices are rejected at mount time
//! because those features change the interpretation of file contents.
//!
//! # Write model and limitations
//!
//! The driver performs in-place allocation rather than F2FS's log-structured
//! relocation. There is no garbage collection and the checkpoint is not
//! rewritten, which keeps the format simple but means the volume is not
//! crash-consistent and `fsck.f2fs` may report stale counters. Hard links,
//! device nodes, extended attributes, and `RENAME_EXCHANGE` are not
//! implemented yet.
//! Files are read and written through the block device directly instead of
//! the page cache, so `mmap` on F2FS files is not supported.
//!
//! # On-disk layout
//!
//! ```text
//! +---------+--------+--------+--------+--------+-------------+
//! | segment | super  | SIT    | NAT    | SSA    | main area   |
//! |   0     | block  | (or)   | (or)   | (or)   | (or)        |
//! +---------+--------+--------+--------+--------+-------------+
//! ```
//!
//! The superblock lives at byte 1024 of the first block and describes the
//! extent of every area. Each area starts with two alternating copies of its
//! metadata; the checkpoint holds one bit per NAT block and one bit per SIT
//! block selecting the copy that is currently valid.
//!
//! # Module structure
//!
//! | Module         | Responsibility                                      |
//! |----------------|-----------------------------------------------------|
//! | `ondisk`       | Packed on-disk structures, offsets, and predicates   |
//! | `alloc`        | SIT allocation, NAT/SSA updates, block writes        |
//! | `fs`           | Mount, checkpoint selection, NAT lookup, block I/O   |
//! | `inode`        | Inode parsing, data mapping, dentries, symlinks      |
//! | `impl_for_vfs` | Wires F2FS into the VFS traits                       |
//! | `fs_type`      | `FsType` registration glue                           |
//!
//! # References
//!
//! - <https://www.kernel.org/doc/html/latest/filesystems/f2fs.html>
//! - Linux `include/linux/f2fs_fs.h`

mod alloc;
mod fs;
mod fs_type;
mod impl_for_vfs;
mod inode;
mod ondisk;
mod prelude;

pub(in crate::fs) use self::fs_type::F2FS_TYPE;
use crate::fs::vfs::registry;

/// Registers the F2FS filesystem type with the VFS registry.
pub(super) fn init() {
    registry::register(&F2FS_TYPE).unwrap();
}
