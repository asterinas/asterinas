// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/sys/kernel/random/boot_id` file support, which provides
//! a random UUID that is generated once at boot time.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc.5.html>

use alloc::format;
use aster_util::printer::VmPrinter;
use spin::Once;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{FileOps, ProcFileBuilder},
        vfs::inode::Inode,
    },
    prelude::*,
    util::random::getrandom,
};

/// Represents the inode at `/proc/sys/kernel/random/boot_id`.
pub struct BootIdFileOps;

impl BootIdFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for BootIdFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        // Get the boot ID (UUID)
        let boot_id = get_boot_id();

        writeln!(printer, "{}", boot_id)?;
        Ok(printer.bytes_written())
    }
}

/// Returns the boot ID as a UUID string.
/// The boot ID is generated once at boot time and remains constant.
fn get_boot_id() -> String {
    static BOOT_ID: Once<String> = Once::new();

    BOOT_ID.call_once(|| {
        // Generate random bytes for UUID
        let mut uuid_bytes = [0u8; 16];
        getrandom(&mut uuid_bytes);

        // Format as UUID string (8-4-4-4-12)
        format_uuid(&uuid_bytes)
    }).to_string()
}

/// Formats 16 bytes as a UUID string (8-4-4-4-12 format)
fn format_uuid(bytes: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}
