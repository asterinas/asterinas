// SPDX-License-Identifier: MPL-2.0

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub enum NVMeRegs32 {
    Vs = 0x8,
    Intms = 0xC,
    Intmc = 0x10,
    Cc = 0x14,
    Csts = 0x1C,
    Nssr = 0x20,
    Aqa = 0x24,
    Cmbloc = 0x38,
    Cmbsz = 0x3C,
    Bpinfo = 0x40,
    Bprsel = 0x44,
    Bpmbl = 0x48,
    Cmbsts = 0x58,
    Pmrcap = 0xE00,
    Pmrctl = 0xE04,
    Pmrsts = 0xE08,
    Pmrebs = 0xE0C,
    Pmrswtp = 0xE10,
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub enum NVMeRegs64 {
    Cap = 0x0,
    Asq = 0x28,
    Acq = 0x30,
    Cmbmsc = 0x50,
    Pmrmsc = 0xE14,
}

#[derive(Copy, Clone, Debug)]
pub enum NVMeDoorBellRegs {
    Sqtdb,
    Cqhdb,
}
