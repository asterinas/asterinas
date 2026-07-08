// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::{
            StaticEntryWithOps,
            template::{
                ListedEntry, ProcDir, ProcDirOps, ProcFile, ProcFileOps, ReaddirEntry,
                listed_entries_from_table, lookup_child_from_table, visit_listed_entries,
            },
        },
        vfs::inode::{Inode, RevalidationPolicy},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    security,
    thread::Thread,
};

const MAX_PROFILE_NAME_LEN: usize = 4096;

/// Represents the inode at `/proc/[pid]/task/[tid]/attr`.
pub(super) struct AttrDirOps(TidDirOps);

impl AttrDirOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L2850>
        ProcDir::new(Self(dir.clone()), parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntryWithOps<AttrDirOps>] = &[
        ("current", InodeType::File, CurrentFileOps::new_inode),
        ("exec", InodeType::File, ExecFileOps::new_inode),
        ("prev", InodeType::File, PrevFileOps::new_inode),
    ];
}

impl ProcDirOps for AttrDirOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if self.0.thread().is_none() {
            return_errno_with_message!(Errno::ENOENT, "the thread does not exist");
        }

        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(self, this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        if self.0.thread().is_none() {
            return_errno_with_message!(Errno::ENOENT, "the thread does not exist");
        }

        visit_listed_entries(offset, self.static_listed_entries(), visit_fn)
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::REVALIDATE_EXISTS
    }

    fn revalidate_exists(&self, _name: &str, _child: &dyn Inode) -> bool {
        self.0.thread().is_some()
    }
}

impl AttrDirOps {
    fn static_listed_entries(&self) -> impl Iterator<Item = ListedEntry<'_>> + '_ {
        listed_entries_from_table(Self::STATIC_ENTRIES)
    }
}

fn task_state_of(dir: &TidDirOps) -> Result<security::AppArmorTaskState> {
    let Some(thread) = dir.thread() else {
        return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
    };
    let posix_thread = thread.as_posix_thread().unwrap();
    let Some(task_state) = security::apparmor_task_state(posix_thread) else {
        return_errno_with_message!(Errno::ENOENT, "the AppArmor LSM is not enabled");
    };

    Ok(task_state)
}

fn ensure_current_thread(thread: &Arc<Thread>) -> Result<()> {
    let Some(current_thread) = Thread::current() else {
        return_errno_with_message!(Errno::ESRCH, "the current thread does not exist");
    };
    if !Arc::ptr_eq(&current_thread, thread) {
        return_errno_with_message!(
            Errno::EPERM,
            "the AppArmor task attribute is only writable by the current thread"
        );
    }

    Ok(())
}

fn read_profile_name_from(reader: &mut VmReader) -> Result<(Option<String>, usize)> {
    let (profile_name, read_bytes) = reader.read_cstring_until_end(MAX_PROFILE_NAME_LEN)?;
    if reader.has_remain() {
        return_errno_with_message!(Errno::E2BIG, "the AppArmor profile name is too large");
    }

    let profile_name = profile_name
        .to_str()
        .map_err(|_| Error::with_message(Errno::EINVAL, "the profile name is not valid UTF-8"))?
        .trim();
    let profile_name = (!profile_name.is_empty()).then(|| profile_name.to_string());

    Ok((profile_name, read_bytes))
}

/// Represents the inode at `/proc/[pid]/task/[tid]/attr/current`.
struct CurrentFileOps(TidDirOps);

impl CurrentFileOps {
    pub fn new_inode(dir: &AttrDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L2775>
        ProcFile::new(Self(dir.0.clone()), parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for CurrentFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let task_state = task_state_of(&self.0)?;
        writeln!(printer, "{}", task_state.current_profile().as_str())?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let Some(thread) = self.0.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
        ensure_current_thread(&thread)?;

        let (profile_name, read_bytes) = read_profile_name_from(reader)?;
        let Some(profile_name) = profile_name else {
            return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name is empty");
        };
        security::set_current_apparmor_profile(&profile_name)?;

        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]/attr/exec`.
struct ExecFileOps(TidDirOps);

impl ExecFileOps {
    pub fn new_inode(dir: &AttrDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L2775>
        ProcFile::new(Self(dir.0.clone()), parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for ExecFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let task_state = task_state_of(&self.0)?;
        if let Some(profile_name) = task_state.onexec_profile() {
            writeln!(printer, "{}", profile_name.as_str())?;
        }

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let Some(thread) = self.0.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
        ensure_current_thread(&thread)?;

        let (profile_name, read_bytes) = read_profile_name_from(reader)?;
        security::set_current_apparmor_onexec_profile(profile_name.as_deref())?;

        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]/attr/prev`.
struct PrevFileOps(TidDirOps);

impl PrevFileOps {
    pub fn new_inode(dir: &AttrDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L2775>
        ProcFile::new(Self(dir.0.clone()), parent, mkmod!(a+r))
    }
}

impl ProcFileOps for PrevFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let task_state = task_state_of(&self.0)?;
        if let Some(profile_name) = task_state.previous_profile() {
            writeln!(printer, "{}", profile_name.as_str())?;
        }

        Ok(printer.bytes_written())
    }
}
