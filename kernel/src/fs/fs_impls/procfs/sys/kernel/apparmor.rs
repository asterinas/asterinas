// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

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
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security,
};

const MAX_POLICY_TEXT_LEN: usize = 64 * 1024;
const MAX_PROFILE_NAME_LEN: usize = 4096;

/// Represents the inode at `/proc/sys/kernel/apparmor`.
pub struct AppArmorDirOps;

impl AppArmorDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/security/apparmor/lsm.c#L2096>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/proc_sysctl.c#L978>
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] = &[
        ("profiles", InodeType::File, ProfilesFileOps::new_inode),
        ("load", InodeType::File, LoadFileOps::new_inode),
        ("current", InodeType::File, CurrentFileOps::new_inode),
    ];
}

impl ProcDirOps for AppArmorDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
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

/// Represents the inode at `/proc/sys/kernel/apparmor/profiles`.
struct ProfilesFileOps;

impl ProfilesFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for ProfilesFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        for (profile_name, mode) in security::apparmor_profile_summaries()? {
            writeln!(printer, "{} {}", profile_name.as_str(), mode.as_str())?;
        }

        Ok(printer.bytes_written())
    }
}

/// Represents the inode at `/proc/sys/kernel/apparmor/load`.
struct LoadFileOps;

impl LoadFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(u+w))
    }
}

impl ProcFileOps for LoadFileOps {
    fn read_at(&self, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "the file is not readable");
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        require_mac_admin()?;

        let (policy_text, read_bytes) = read_text_from(reader, MAX_POLICY_TEXT_LEN)?;
        security::load_apparmor_policy(&policy_text)?;

        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/sys/kernel/apparmor/current`.
struct CurrentFileOps;

impl CurrentFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for CurrentFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let current_thread = current_thread!();
        let Some(posix_thread) = current_thread.as_posix_thread() else {
            return_errno_with_message!(Errno::ESRCH, "the current thread is not a POSIX thread");
        };
        let Some(task_state) = security::apparmor_task_state(posix_thread) else {
            return_errno_with_message!(Errno::ENOENT, "the AppArmor LSM is not enabled");
        };

        writeln!(printer, "{}", task_state.current_profile().as_str())?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        require_mac_admin()?;

        let (profile_name, read_bytes) = read_text_from(reader, MAX_PROFILE_NAME_LEN)?;
        let profile_name = profile_name.trim();
        if profile_name.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name is empty");
        }

        security::set_current_apparmor_profile(profile_name)?;

        Ok(read_bytes)
    }
}

fn require_mac_admin() -> Result<()> {
    let current_thread = current_thread!();
    let Some(posix_thread) = current_thread.as_posix_thread() else {
        return_errno_with_message!(Errno::ESRCH, "the current thread is not a POSIX thread");
    };

    security::capable(
        UserNamespace::get_init_singleton().as_ref(),
        CapSet::MAC_ADMIN,
        posix_thread,
    )
}

fn read_text_from(reader: &mut VmReader, max_len: usize) -> Result<(String, usize)> {
    let (text, read_bytes) = reader.read_cstring_until_end(max_len)?;
    if reader.has_remain() {
        return_errno_with_message!(Errno::E2BIG, "the AppArmor policy text is too large");
    }

    let text = text
        .to_str()
        .map_err(|_| Error::with_message(Errno::EINVAL, "the text is not valid UTF-8"))?;

    Ok((text.to_string(), read_bytes))
}
