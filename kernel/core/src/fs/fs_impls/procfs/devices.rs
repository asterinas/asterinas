// SPDX-License-Identifier: MPL-2.0

//! Registered character and block driver names exposed by `/proc/devices`.
//!
//! Rows describe number reservations, not individual device nodes. Character
//! drivers may register disjoint minor ranges under the same major.

use aster_util::printer::VmPrinter;

use crate::{
    device::registry::char,
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
};

/// Read-only enumeration of registered device-number owners.
pub(super) struct DevicesFileOps;

impl DevicesFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for DevicesFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        // As in Linux 6.6 (5078a3a23a1e), fs/proc/devices.c, character and block
        // registrations occupy separate sections. Neither snapshot holds a
        // registry lock while copying to userspace.
        let characters = char::major_devices();
        let blocks = aster_block::major_devices();
        let mut printer = VmPrinter::new_skip(writer, offset);
        writeln!(printer, "Character devices:")?;
        for (major, name) in characters {
            writeln!(printer, "{major:3} {name}")?;
        }
        writeln!(printer)?;
        writeln!(printer, "Block devices:")?;
        for (major, name) in blocks {
            writeln!(printer, "{major:3} {name}")?;
        }
        Ok(printer.bytes_written())
    }
}

#[cfg(ktest)]
mod tests {
    use device_id::MajorId;
    use ostd::prelude::ktest;

    use super::*;

    fn read(offset: usize, buffer: &mut [u8]) -> usize {
        DevicesFileOps
            .read_at(offset, &mut VmWriter::from(buffer).to_fallible())
            .unwrap()
    }

    #[ktest]
    fn lists_both_registries_and_preserves_shared_character_majors() {
        let major = MajorId::new(506);
        let first = char::acquire_region(major, 0..1, "proc-first").unwrap();
        let second = char::acquire_region(major, 1..2, "proc-second").unwrap();
        let block = aster_block::acquire_major_with_name(major, "proc-block").unwrap();
        let mut buffer = [0u8; 8192];
        let len = read(0, &mut buffer);
        let text = core::str::from_utf8(&buffer[..len]).unwrap();
        assert!(text.starts_with("Character devices:\n"));
        let (characters, blocks) = text.split_once("\nBlock devices:\n").unwrap();
        assert!(characters.contains("506 proc-first\n506 proc-second\n"));
        assert!(blocks.contains("506 proc-block\n"));
        assert!(!characters.contains("proc-block"));
        assert!(!blocks.contains("proc-first"));
        drop(first);
        drop(second);
        drop(block);
        let len = read(0, &mut buffer);
        let text = core::str::from_utf8(&buffer[..len]).unwrap();
        assert!(!text.contains("proc-first"));
        assert!(!text.contains("proc-second"));
        assert!(!text.contains("proc-block"));
    }

    #[ktest]
    fn partial_reads_offsets_and_eof_match_a_full_read() {
        let mut full = [0u8; 8192];
        let len = read(0, &mut full);
        assert!(len < full.len());
        let mut chunk = [0u8; 7];
        let mut offset = 0;
        while offset < len {
            let count = read(offset, &mut chunk);
            assert_eq!(count, chunk.len().min(len - offset));
            assert_eq!(&chunk[..count], &full[offset..offset + count]);
            offset += count;
        }
        assert_eq!(read(len, &mut chunk), 0);
        assert_eq!(read(len + 10, &mut chunk), 0);
        assert_eq!(read(0, &mut []), 0);
    }
}
