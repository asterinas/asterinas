// SPDX-License-Identifier: MPL-2.0

use core::fmt::Write;

use aster_util::printer::VmPrinter;
use spin::Once;

use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::{
            StaticEntry,
            template::{
                ProcDir, ProcDirOps, ProcFile, ProcFileOps, ReaddirEntry,
                listed_entries_from_table, lookup_child_from_table, visit_listed_entries,
            },
        },
        vfs::inode::Inode,
    },
    prelude::*,
    util::random::getrandom,
};

/// The UUID that identifies the current boot.
///
/// It is generated on the first read and remains unchanged afterwards.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/Documentation/admin-guide/sysctl/kernel.rst#L1147>.
static BOOT_ID: Once<Uuid> = Once::new();

/// Represents the inode at `/proc/sys/kernel/random`.
pub(super) struct RandomDirOps;

impl RandomDirOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/proc_sysctl.c#L978>
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] =
        &[("boot_id", InodeType::File, BootIdFileOps::new_inode)];
}

impl ProcDirOps for RandomDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        Err(errno!(ENOENT, "the file does not exist"))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        visit_listed_entries(
            offset,
            listed_entries_from_table(Self::STATIC_ENTRIES),
            visit_fn,
        )
    }
}

/// Represents the inode at `/proc/sys/kernel/random/boot_id`.
struct BootIdFileOps;

impl BootIdFileOps {
    fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/drivers/char/random.c#L1702>
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for BootIdFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        write!(printer, "{}", boot_id())?;
        writeln!(printer)?;

        Ok(printer.bytes_written())
    }
}

fn boot_id() -> &'static Uuid {
    BOOT_ID.call_once(|| {
        let mut uuid = [0u8; 16];
        getrandom(&mut uuid);
        // Set the UUID version to 4 and the UUID variant to DCE, as Linux does.
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/lib/uuid.c#L33-L40>
        uuid[6] = (uuid[6] & 0x0f) | 0x40;
        uuid[8] = (uuid[8] & 0x3f) | 0x80;
        Uuid(uuid)
    });
    BOOT_ID.get().unwrap()
}

/// A UUID displayed in the canonical 8-4-4-4-12 "hex-and-dash" form with
/// lowercase hexadecimal digits.
///
/// Reference: <https://www.rfc-editor.org/rfc/rfc9562.html#section-4>
struct Uuid([u8; 16]);

impl core::fmt::Display for Uuid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_char('-')?;
            }
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}
