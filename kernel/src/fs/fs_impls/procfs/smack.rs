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
    security,
};

const MAX_POLICY_WRITE_LEN: usize = 64 * 1024;

/// Represents the inode at `/proc/smack`.
pub struct SmackDirOps;

impl SmackDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] = &[
        ("load", InodeType::File, LoadFileOps::new_inode),
        ("load2", InodeType::File, LoadFileOps::new_inode),
        ("access", InodeType::File, AccessFileOps::new_inode),
        ("access2", InodeType::File, AccessFileOps::new_inode),
        ("accesses", InodeType::File, AccessesFileOps::new_inode),
        ("change-rule", InodeType::File, ChangeRuleFileOps::new_inode),
        (
            "revoke-subject",
            InodeType::File,
            RevokeSubjectFileOps::new_inode,
        ),
        ("ambient", InodeType::File, AmbientFileOps::new_inode),
        ("onlycap", InodeType::File, OnlycapFileOps::new_inode),
        ("logging", InodeType::File, LoggingFileOps::new_inode),
    ];
}

impl ProcDirOps for SmackDirOps {
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

/// Represents the inode at `/proc/smack/load`.
struct LoadFileOps;

impl LoadFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for LoadFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        write_rules_at(offset, writer)
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        read_and_load_rules(reader)
    }
}

/// Represents the inode at `/proc/smack/access`.
struct AccessFileOps;

impl AccessFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for AccessFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        write_text_at(
            offset,
            writer,
            security::smack_access_query_result_as_text()?,
        )
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (query, read_bytes) = read_policy_text(reader)?;
        security::query_smack_access(&query)?;
        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/smack/accesses`.
struct AccessesFileOps;

impl AccessesFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for AccessesFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        write_rules_at(offset, writer)
    }
}

/// Represents the inode at `/proc/smack/change-rule`.
struct ChangeRuleFileOps;

impl ChangeRuleFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(u+w))
    }
}

impl ProcFileOps for ChangeRuleFileOps {
    fn read_at(&self, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "the Smack change-rule file is write-only");
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (rule, read_bytes) = read_policy_text(reader)?;
        security::change_smack_rule(&rule)?;
        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/smack/revoke-subject`.
struct RevokeSubjectFileOps;

impl RevokeSubjectFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(u+w))
    }
}

impl ProcFileOps for RevokeSubjectFileOps {
    fn read_at(&self, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "the Smack revoke-subject file is write-only");
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (subject, read_bytes) = read_policy_text(reader)?;
        security::revoke_smack_subject(&subject)?;
        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/smack/ambient`.
struct AmbientFileOps;

impl AmbientFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for AmbientFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        write_text_at(offset, writer, security::smack_ambient_label_as_text()?)
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (label, read_bytes) = read_policy_text(reader)?;
        security::set_smack_ambient_label(&label)?;
        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/smack/onlycap`.
struct OnlycapFileOps;

impl OnlycapFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for OnlycapFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        write_text_at(offset, writer, security::smack_onlycap_labels_as_text()?)
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (labels, read_bytes) = read_policy_text(reader)?;
        security::set_smack_onlycap_labels(&labels)?;
        Ok(read_bytes)
    }
}

/// Represents the inode at `/proc/smack/logging`.
struct LoggingFileOps;

impl LoggingFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for LoggingFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        write_text_at(offset, writer, security::smack_logging_mode_as_text()?)
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        let (mode, read_bytes) = read_policy_text(reader)?;
        security::set_smack_logging_mode(&mode)?;
        Ok(read_bytes)
    }
}

fn write_rules_at(offset: usize, writer: &mut VmWriter) -> Result<usize> {
    write_text_at(offset, writer, security::smack_rules_as_text()?)
}

fn write_text_at(offset: usize, writer: &mut VmWriter, text: String) -> Result<usize> {
    let mut printer = VmPrinter::new_skip(writer, offset);
    write!(printer, "{}", text)?;
    Ok(printer.bytes_written())
}

fn read_and_load_rules(reader: &mut VmReader) -> Result<usize> {
    let (policy, read_bytes) = read_policy_text(reader)?;
    security::load_smack_rules(&policy)?;

    Ok(read_bytes)
}

fn read_policy_text(reader: &mut VmReader) -> Result<(String, usize)> {
    let read_bytes = reader.remain();
    if read_bytes > MAX_POLICY_WRITE_LEN {
        return_errno_with_message!(Errno::E2BIG, "the Smack policy write is too large");
    }

    let mut policy = vec![0u8; read_bytes];
    reader.read_fallible(&mut VmWriter::from(policy.as_mut_slice()))?;
    let policy = core::str::from_utf8(&policy)
        .map_err(|_| Error::with_message(Errno::EINVAL, "the Smack policy is not UTF-8"))?;
    Ok((policy.to_string(), read_bytes))
}
