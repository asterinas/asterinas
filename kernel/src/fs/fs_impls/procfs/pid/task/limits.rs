// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    process::ResourceType,
    thread::Thread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/limits` and `/proc/[pid]/limits`.
pub struct LimitsFileOps(TidDirOps);

struct LimitSpec {
    resource: ResourceType,
    name: &'static str,
    unit: &'static str,
}

const LIMIT_SPECS: &[LimitSpec] = &[
    LimitSpec {
        resource: ResourceType::RLIMIT_CPU,
        name: "Max cpu time",
        unit: "seconds",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_FSIZE,
        name: "Max file size",
        unit: "bytes",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_DATA,
        name: "Max data size",
        unit: "bytes",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_STACK,
        name: "Max stack size",
        unit: "bytes",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_CORE,
        name: "Max core file size",
        unit: "bytes",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_RSS,
        name: "Max resident set",
        unit: "bytes",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_NPROC,
        name: "Max processes",
        unit: "processes",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_NOFILE,
        name: "Max open files",
        unit: "files",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_MEMLOCK,
        name: "Max locked memory",
        unit: "bytes",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_AS,
        name: "Max address space",
        unit: "bytes",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_LOCKS,
        name: "Max file locks",
        unit: "locks",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_SIGPENDING,
        name: "Max pending signals",
        unit: "signals",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_MSGQUEUE,
        name: "Max msgqueue size",
        unit: "bytes",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_NICE,
        name: "Max nice priority",
        unit: "",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_RTPRIO,
        name: "Max realtime priority",
        unit: "",
    },
    LimitSpec {
        resource: ResourceType::RLIMIT_RTTIME,
        name: "Max realtime timeout",
        unit: "us",
    },
];

impl LimitsFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3332>
        ProcFile::new(Self(dir.clone()), parent, mkmod!(a+r))
    }
}

impl ProcFileOps for LimitsFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        let Some(process) = self.0.process() else {
            return_errno_with_message!(Errno::ESRCH, "the process does not exist");
        };

        writeln!(
            printer,
            "{:<25} {:<20} {:<20} Units",
            "Limit", "Soft Limit", "Hard Limit"
        )?;
        for spec in LIMIT_SPECS {
            let rlimit = process
                .resource_limits()
                .get_rlimit(spec.resource)
                .get_raw_rlimit();
            write!(printer, "{:<25} ", spec.name)?;
            write_limit_value(&mut printer, rlimit.cur)?;
            write!(printer, " ")?;
            write_limit_value(&mut printer, rlimit.max)?;
            writeln!(printer, " {}", spec.unit)?;
        }

        Ok(printer.bytes_written())
    }
}

fn write_limit_value(printer: &mut VmPrinter, value: u64) -> Result<()> {
    if value == u64::MAX {
        write!(printer, "{:<20}", "unlimited")?;
    } else {
        write!(printer, "{:<20}", value)?;
    }
    Ok(())
}
