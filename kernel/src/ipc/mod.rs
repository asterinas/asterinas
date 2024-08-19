// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    process::{Gid, Uid},
};

pub mod semaphore;

#[allow(non_camel_case_types)]
pub type key_t = i32;

bitflags! {
    pub struct IpcFlags: u32{
        /// Create key if key does not exist
        const IPC_CREAT  = 1 << 9;
        /// Fail if key exists
        const IPC_EXCL  = 1 << 10;
        /// Return error on wait
        const IPC_NOWAIT  = 1 << 11;
        /// Undo the operation on exit
        const SEM_UNDO = 1 << 12;
    }
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
pub enum IpcControlCmd {
    IPC_RMID = 0,
    IPC_SET = 1,
    IPC_STAT = 2,

    SEM_GETPID = 11,
    SEM_GETVAL = 12,
    SEM_GETALL = 13,
    SEM_GETNCNT = 14,
    SEM_GETZCNT = 15,
    SEM_SETVAL = 16,
    SEM_SETALL = 17,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct IpcPermission {
    key: key_t,
    /// Owner's UID
    uid: Uid,
    /// Owner's GID
    gid: Gid,
    /// Creator's UID
    cuid: Uid,
    /// Creator's GID
    cguid: Gid,
    /// Permission mode
    mode: u16,
}

impl IpcPermission {
    pub fn key(&self) -> key_t {
        self.key
    }

    /// Returns owner's UID
    pub fn uid(&self) -> Uid {
        self.uid
    }

    /// Returns owner's GID
    pub fn gid(&self) -> Gid {
        self.gid
    }

    /// Returns creator's UID
    pub fn cuid(&self) -> Uid {
        self.cuid
    }

    /// Returns creator's GID
    pub fn cguid(&self) -> Gid {
        self.cguid
    }

    /// Returns permission mode
    pub fn mode(&self) -> u16 {
        self.mode
    }

    pub(self) fn new_sem_perm(key: key_t, uid: Uid, gid: Gid, mode: u16) -> Self {
        Self {
            key,
            uid,
            gid,
            cuid: uid,
            cguid: gid,
            mode,
        }
    }
}

pub(super) fn init() {
    semaphore::init();
}
