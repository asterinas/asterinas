// SPDX-License-Identifier: MPL-2.0

//! This mod defines the types used in IPC operations

use ostd::Pod;

/// IPC permissions structure
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct IpcPerm {
    /// Key
    pub key: i32,
    /// Owner's user ID
    pub uid: u32,
    /// Owner's group ID
    pub gid: u32,
    /// Creator's user ID
    pub cuid: u32,
    /// Creator's group ID
    pub cgid: u32,
    /// Read/write permission
    pub mode: u16,
    /// Sequence number
    pub seq: u16,
    /// Padding
    pub _pad2: u16,
    /// Reserved for future use
    pub _glibc_reserved1: u64,
    /// Reserved for future use
    pub _glibc_reserved2: u64,
}

/// Shared memory segment data structure
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct ShmidDs {
    /// Operation permissions
    pub shm_perm: IpcPerm,
    /// Size of segment in bytes
    pub shm_segsz: usize,
    /// Last attach time
    pub shm_atime: i64,
    /// Last detach time
    pub shm_dtime: i64,
    /// Last change time
    pub shm_ctime: i64,
    /// PID of creator
    pub shm_cpid: i32,
    /// PID of last operator
    pub shm_lpid: i32,
    /// Number of current attaches
    pub shm_nattch: u64,
    /// Reserved for future use
    pub _glibc_reserved5: u64,
    /// Reserved for future use
    pub _glibc_reserved6: u64,
}
