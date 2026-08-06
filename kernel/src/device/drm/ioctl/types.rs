// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

#[repr(C)]
#[padding_struct]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmVersion {
    pub version_major: i32,
    pub version_minor: i32,
    pub version_patchlevel: i32,

    pub name_len: usize,
    pub name: usize,
    pub date_len: usize,
    pub date: usize,
    pub desc_len: usize,
    pub desc: usize,
}

#[repr(u64)]
#[derive(Debug, TryFromInt)]
pub enum DrmGetCapability {
    DumbBuffer = 0x1,
    VblankHighCrtc = 0x2,
    DumbPreferredDepth = 0x3,
    DumbPreferShadow = 0x4,
    Prime = 0x5,
    TimestampMonotonic = 0x6,
    AsyncPageFlip = 0x7,
    CursorWidth = 0x8,
    CursorHeight = 0x9,
    Addfb2Modifiers = 0x10,
    PageFlipTarget = 0x11,
    CrtcInVblankEvent = 0x12,
    SyncObj = 0x13,
    SyncObjTimeline = 0x14,
    AtomicAsyncPageFlip = 0x15,
}

bitflags::bitflags! {
    pub struct DrmPrimeValue: u64 {
        const IMPORT = 0x1;
        const EXPORT = 0x2;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmGetCap {
    pub capability: u64,
    pub value: u64,
}

#[repr(u64)]
#[derive(Debug, TryFromInt)]
pub enum DrmSetCapability {
    Stereo3D = 0x1,
    UniversalPlane = 0x2,
    Atomic = 0x3,
    AspectRatio = 0x4,
    WritebackConnectors = 0x5,
    CursorPlaneHostport = 0x6,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmSetClientCap {
    pub capability: u64,
    pub value: u64,
}
