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

const MAX_WRITTEN_LABEL_LEN: usize = 256;

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
        ("fscreate", InodeType::File, FscreateFileOps::new_inode),
        ("sockcreate", InodeType::File, SockcreateFileOps::new_inode),
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

fn task_state_of(dir: &TidDirOps) -> Result<security::SmackTaskState> {
    let Some(thread) = dir.thread() else {
        return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
    };
    let Some(posix_thread) = thread.as_posix_thread() else {
        return_errno_with_message!(Errno::ESRCH, "the thread is not a POSIX thread");
    };
    let Some(task_state) = security::smack_task_state(posix_thread) else {
        return_errno_with_message!(Errno::ENOENT, "the Smack LSM is not enabled");
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
            "the Smack task attribute is only writable by the current thread"
        );
    }

    Ok(())
}

fn read_label_from(reader: &mut VmReader) -> Result<(String, usize)> {
    let (label, read_bytes) = reader.read_cstring_until_end(MAX_WRITTEN_LABEL_LEN)?;
    if reader.has_remain() {
        return_errno_with_message!(Errno::E2BIG, "the Smack label is too large");
    }

    let label = label
        .to_str()
        .map_err(|_| Error::with_message(Errno::EINVAL, "the Smack label is not valid UTF-8"))?
        .trim();
    if label.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the Smack label is empty");
    }

    Ok((label.to_string(), read_bytes))
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
        writeln!(printer, "{}", task_state.current_label().as_str())?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let Some(thread) = self.0.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
        ensure_current_thread(&thread)?;

        let (label, read_bytes) = read_label_from(reader)?;
        security::set_current_smack_label(&label)?;

        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]/attr/exec`.
struct ExecFileOps(TidDirOps);

impl ExecFileOps {
    pub fn new_inode(dir: &AttrDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir.0.clone()), parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for ExecFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        if let Some(label) = task_state_of(&self.0)?.exec_label() {
            write!(printer, "{}", label.as_str())?;
        }
        writeln!(printer)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let Some(thread) = self.0.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
        ensure_current_thread(&thread)?;

        let (label, read_bytes) = read_label_from(reader)?;
        security::set_current_smack_exec_label(&label)?;

        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]/attr/prev`.
struct PrevFileOps(TidDirOps);

impl PrevFileOps {
    pub fn new_inode(dir: &AttrDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir.0.clone()), parent, mkmod!(a+r))
    }
}

impl ProcFileOps for PrevFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        if let Some(label) = task_state_of(&self.0)?.previous_label() {
            write!(printer, "{}", label.as_str())?;
        }
        writeln!(printer)?;

        Ok(printer.bytes_written())
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]/attr/fscreate`.
struct FscreateFileOps(TidDirOps);

impl FscreateFileOps {
    pub fn new_inode(dir: &AttrDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir.0.clone()), parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for FscreateFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        if let Some(label) = task_state_of(&self.0)?.fscreate_label() {
            write!(printer, "{}", label.as_str())?;
        }
        writeln!(printer)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let Some(thread) = self.0.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
        ensure_current_thread(&thread)?;

        let (label, read_bytes) = read_label_from(reader)?;
        security::set_current_smack_fscreate_label(&label)?;

        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]/attr/sockcreate`.
struct SockcreateFileOps(TidDirOps);

impl SockcreateFileOps {
    pub fn new_inode(dir: &AttrDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir.0.clone()), parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for SockcreateFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        if let Some(label) = task_state_of(&self.0)?.sockcreate_label() {
            write!(printer, "{}", label.as_str())?;
        }
        writeln!(printer)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let Some(thread) = self.0.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
        ensure_current_thread(&thread)?;

        let (label, read_bytes) = read_label_from(reader)?;
        security::set_current_smack_sockcreate_label(&label)?;

        Ok(read_bytes)
    }
}
