// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use crate::prelude::*;

/// Auxiliary Vector.
///
/// # What is Auxiliary Vector?
///
/// Here is a concise description of Auxiliary Vector from GNU's manual:
///
///  > When a program is executed, it receives information from the operating system
///  > about the environment in which it is operating. The form of this information
///  > is a table of key-value pairs, where the keys are from the set of ‘AT_’
///  > values in elf.h.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum AuxKey {
    AT_NULL = 0,      /* end of vector */
    AT_IGNORE = 1,    /* entry should be ignored */
    AT_EXECFD = 2,    /* file descriptor of program */
    AT_PHDR = 3,      /* program headers for program */
    AT_PHENT = 4,     /* size of program header entry */
    AT_PHNUM = 5,     /* number of program headers */
    AT_PAGESZ = 6,    /* system page size */
    AT_BASE = 7,      /* base address of interpreter */
    AT_FLAGS = 8,     /* flags */
    AT_ENTRY = 9,     /* entry point of program */
    AT_NOTELF = 10,   /* program is not ELF */
    AT_UID = 11,      /* real uid */
    AT_EUID = 12,     /* effective uid */
    AT_GID = 13,      /* real gid */
    AT_EGID = 14,     /* effective gid */
    AT_PLATFORM = 15, /* string identifying CPU for optimizations */
    AT_HWCAP = 16,    /* arch dependent hints at CPU capabilities */
    AT_CLKTCK = 17,   /* frequency at which times() increments */

    /* 18...22 not used */
    AT_SECURE = 23, /* secure mode boolean */
    AT_BASE_PLATFORM = 24, /* string identifying real platform, may
                     * differ from AT_PLATFORM. */
    AT_RANDOM = 25, /* address of 16 random bytes */
    AT_HWCAP2 = 26, /* extension of AT_HWCAP */

    /* 28...30 not used */
    AT_EXECFN = 31, /* filename of program */
    AT_SYSINFO = 32,
    AT_SYSINFO_EHDR = 33, /* the start address of the page containing the VDSO */
}

impl AuxKey {
    pub fn as_u64(&self) -> u64 {
        *self as u64
    }
}

#[derive(Clone, Default, Debug)]
pub struct AuxVec {
    table: BTreeMap<AuxKey, u64>,
}

impl AuxVec {
    pub const fn new() -> AuxVec {
        AuxVec {
            table: BTreeMap::new(),
        }
    }
}

impl AuxVec {
    pub fn set(&mut self, key: AuxKey, val: u64) -> Result<()> {
        if key == AuxKey::AT_NULL || key == AuxKey::AT_IGNORE {
            return_errno_with_message!(Errno::EINVAL, "Illegal key");
        }
        self.table
            .entry(key)
            .and_modify(|val_mut| *val_mut = val)
            .or_insert(val);
        Ok(())
    }

    pub fn get(&self, key: AuxKey) -> Option<u64> {
        self.table.get(&key).copied()
    }

    pub fn del(&mut self, key: AuxKey) -> Option<u64> {
        self.table.remove(&key)
    }

    pub fn table(&self) -> &BTreeMap<AuxKey, u64> {
        &self.table
    }
}
