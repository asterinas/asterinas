// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;
use ostd::task::Task;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    net::uts_ns::{UtsField, UtsName, UtsNamespace},
    prelude::*,
    process::posix_thread::{AsPosixThread, PosixThread},
};

/// Represents the inode at `/proc/sys/kernel/hostname`.
pub struct HostnameFileOps;

impl HostnameFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/utsname_sysctl.c#L108>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for HostnameFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        read_current_uts_name(offset, writer, UtsName::nodename)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        write_current_uts_name(
            offset,
            reader,
            UtsName::nodename,
            UtsNamespace::set_hostname,
        )
    }
}

/// Represents the inode at `/proc/sys/kernel/domainname`.
pub struct DomainnameFileOps;

impl DomainnameFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/utsname_sysctl.c#L116>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for DomainnameFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        read_current_uts_name(offset, writer, UtsName::domainname)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        write_current_uts_name(
            offset,
            reader,
            UtsName::domainname,
            UtsNamespace::set_domainname,
        )
    }
}

/// Represents the inode at `/proc/sys/kernel/osrelease`.
pub struct OsReleaseFileOps;

impl OsReleaseFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/utsname_sysctl.c#L94>
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for OsReleaseFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        read_uts_bytes(offset, writer, UtsName::RELEASE.as_bytes())
    }
}

/// Represents the inode at `/proc/sys/kernel/version`.
pub struct VersionFileOps;

impl VersionFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/utsname_sysctl.c#L101>
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for VersionFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        read_uts_bytes(offset, writer, UtsName::VERSION.as_bytes())
    }
}

fn read_current_uts_name(
    offset: usize,
    writer: &mut VmWriter,
    get_value: fn(&UtsName) -> &UtsField,
) -> Result<usize> {
    let current_task = Task::current().unwrap();
    let thread_local = current_task.as_thread_local().unwrap();
    let ns_proxy = thread_local.borrow_ns_proxy();

    let value = {
        let uts_name = ns_proxy.unwrap().uts_ns().uts_name();
        *get_value(&uts_name)
    };

    read_uts_bytes(offset, writer, value.as_cstr().to_bytes())
}

fn read_uts_bytes(offset: usize, writer: &mut VmWriter, value: &[u8]) -> Result<usize> {
    let mut printer = VmPrinter::new_skip(writer, offset);

    printer.write_bytes(value)?;
    printer.write_bytes(b"\n")?;

    Ok(printer.bytes_written())
}

fn write_current_uts_name(
    offset: usize,
    reader: &mut VmReader,
    get_value: fn(&UtsName) -> &UtsField,
    set_value: fn(&UtsNamespace, UtsField, &PosixThread) -> Result<()>,
) -> Result<usize> {
    let len = reader.remain();
    if offset >= UtsField::MAX_BYTES || len == 0 {
        reader.skip(len);
        return Ok(len);
    }

    let current_task = Task::current().unwrap();
    let posix_thread = current_task.as_posix_thread().unwrap();

    let thread_local = current_task.as_thread_local().unwrap();
    let ns_proxy = thread_local.borrow_ns_proxy();
    let uts_ns = ns_proxy.unwrap().uts_ns();

    // We need to edit the old name. The lock is released before writing the
    // new value, so concurrent updates can race. Linux does the same thing:
    // <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/utsname_sysctl.c#L57-L59>.
    let mut value = if offset != 0 {
        let current_uts_name = uts_ns.uts_name();
        let current_value = get_value(&current_uts_name);
        *current_value.as_array()
    } else {
        [0u8; UtsField::MAX_BYTES_WITH_NUL]
    };

    let mut writer = VmWriter::from(&mut value[offset..]).to_fallible();
    let copied_len = reader.read_fallible(&mut writer)?;

    // Truncate at the first newline. Any bytes after the first nul byte will be
    // zeroed by `UtsField::from_bytes_until_nul`.
    for byte in &mut value[offset..offset + copied_len] {
        if *byte == b'\n' {
            *byte = 0;
            break;
        }
    }

    let new_value = UtsField::from_bytes_until_nul(&value);
    set_value(uts_ns, new_value, posix_thread)?;

    reader.skip(reader.remain());
    Ok(len)
}
