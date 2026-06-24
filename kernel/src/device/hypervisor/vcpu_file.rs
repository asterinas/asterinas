// SPDX-License-Identifier: MPL-2.0

//! VCPU file descriptor implementation

use ostd::{
    arch::vm::{GuestExitInfo, VmxExitReason},
    mm::VmIo,
    task::Task,
    vm::GuestMode,
};

use super::{
    apic::emulate_apic_mmio,
    ioctl::*,
    vcpu::{PendingMmioOperation, PendingOperation, PendingPioOperation, PioDirection},
    vm::Vm,
};
use crate::{
    fs::{
        file::{AccessMode, FileLike, Mappable, file_table::FdFlags},
        pseudofs::AnonInodeFs,
        vfs::path::Path,
    },
    prelude::*,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

// Periodically return timer exits so a busy vCPU cannot monopolize scheduling.
const PREEMPTION_TIMER_USER_EXIT_INTERVAL: u64 = 16;
const GPR_RAX: u8 = 0;

pub(super) use super::vcpu::Vcpu;

/// VCPU file descriptor
pub struct VcpuFile {
    vm: Arc<Vm>,
    vcpu: Arc<Vcpu>,
    pseudo_path: Path,
}

impl VcpuFile {
    /// Creates a new VCPU file
    pub fn new(vm: Arc<Vm>, vcpu: Arc<Vcpu>) -> Self {
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[rustshyper-vcpu]".to_string());
        Self {
            vm,
            vcpu,
            pseudo_path,
        }
    }
}

impl FileLike for VcpuFile {
    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read from VCPU file");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write to VCPU file");
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            Run => {
                self.ioctl_run()
            }
            cmd @ GetRegs => {
                let regs = self.vcpu.get_regs()?;
                cmd.write(&regs)?;
                Ok(0)
            }
            cmd @ SetRegs => {
                let regs = cmd.read()?;
                self.vcpu.set_regs(regs)?;
                Ok(0)
            }
            cmd @ GetSregs => {
                let sregs = self.vcpu.get_sregs()?;
                cmd.write(&sregs)?;
                Ok(0)
            }
            cmd @ SetSregs => {
                let sregs = cmd.read()?;
                self.vcpu.set_sregs(sregs)?;
                Ok(0)
            }
            cmd @ GetMsrs => {
                let msrs = cmd.with_data_ptr(|ptr| Ok(ptr.read()?))?;
                let mut entries = read_msr_entries(msrs, raw_ioctl.arg())?;
                let handled_count = self.vcpu.get_msrs(&mut entries)?;
                write_msr_entries(msrs, raw_ioctl.arg(), &entries)?;
                Ok(handled_count)
            }
            cmd @ SetMsrs => {
                let msrs = cmd.read()?;
                let entries = read_msr_entries(msrs, raw_ioctl.arg())?;
                self.vcpu.set_msrs(&entries)
            }
            cmd @ SetFpu => {
                let _fpu = cmd.read()?;
                // TODO: Install FPU/XMM state into the guest context.
                Ok(0)
            }
            cmd @ GetLapic => {
                let lapic = self.vcpu.get_lapic()?;
                cmd.write(&lapic)?;
                Ok(0)
            }
            cmd @ SetLapic => {
                let lapic = cmd.read()?;
                self.vcpu.set_lapic(&lapic)?;
                Ok(0)
            }
            cmd @ SetCpuid2 => {
                let cpuid = cmd.read()?;
                let entries = read_cpuid_entries(cpuid, raw_ioctl.arg())?;
                self.vcpu.set_cpuid_entries(entries)?;
                Ok(0)
            }
            _ => {
                let ioctl_nr = raw_ioctl.cmd() & 0xff;
                error!(
                    "rustshyper: unimplemented VCPU ioctl command: cmd={:#x}, nr={:#x}",
                    raw_ioctl.cmd(),
                    ioctl_nr
                );
                return_errno_with_message!(Errno::ENOTTY, "unknown VCPU ioctl command");
            }
        })
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn mappable(&self) -> Result<Mappable> {
        Ok(Mappable::Vmo(self.vcpu.run_page()))
    }

    fn dump_proc_fdinfo(self: Arc<Self>, _fd_flags: FdFlags) -> Box<dyn core::fmt::Display> {
        Box::new("rustshyper_vcpu\n")
    }
}

fn read_cpuid_entries(cpuid: VcpuCpuid2, arg: usize) -> Result<Vec<VcpuCpuidEntry2>> {
    let nent = usize::try_from(cpuid.nent)?;
    if nent > KVM_MAX_CPUID_ENTRIES {
        return_errno_with_message!(Errno::E2BIG, "too many CPUID entries");
    }

    let entries_addr = arg
        .checked_add(size_of::<VcpuCpuid2>())
        .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
    let entries_len = nent
        .checked_mul(size_of::<VcpuCpuidEntry2>())
        .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
    let current = Task::current().unwrap();
    let thread_local = current.as_thread_local().unwrap();
    let user_space = CurrentUserSpace::new(thread_local);
    let mut reader = user_space.reader(entries_addr, entries_len)?;
    let mut entries = Vec::new();
    for _ in 0..nent {
        entries.push(reader.read_val()?);
    }

    Ok(entries)
}

fn read_msr_entries(msrs: VcpuMsrs, arg: usize) -> Result<Vec<VcpuMsrEntry>> {
    let nmsrs = usize::try_from(msrs.nmsrs)?;
    if nmsrs > KVM_MAX_MSR_ENTRIES {
        return_errno_with_message!(Errno::E2BIG, "too many MSR entries");
    }

    let entries_addr = arg
        .checked_add(size_of::<VcpuMsrs>())
        .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
    let entries_len = nmsrs
        .checked_mul(size_of::<VcpuMsrEntry>())
        .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
    let current = Task::current().unwrap();
    let thread_local = current.as_thread_local().unwrap();
    let user_space = CurrentUserSpace::new(thread_local);
    let mut reader = user_space.reader(entries_addr, entries_len)?;
    let mut entries = Vec::new();
    for _ in 0..nmsrs {
        entries.push(reader.read_val()?);
    }

    Ok(entries)
}

fn write_msr_entries(msrs: VcpuMsrs, arg: usize, entries: &[VcpuMsrEntry]) -> Result<()> {
    debug_assert_eq!(usize::try_from(msrs.nmsrs).ok(), Some(entries.len()));

    let entries_addr = arg
        .checked_add(size_of::<VcpuMsrs>())
        .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
    let entries_len = entries
        .len()
        .checked_mul(size_of::<VcpuMsrEntry>())
        .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
    let current = Task::current().unwrap();
    let thread_local = current.as_thread_local().unwrap();
    let user_space = CurrentUserSpace::new(thread_local);
    user_space.write_val(arg, &msrs)?;
    let mut writer = user_space.writer(entries_addr, entries_len)?;
    for entry in entries {
        writer.write_val(entry)?;
    }

    Ok(())
}

impl VcpuFile {
    fn ioctl_run(&self) -> Result<i32> {
        self.complete_pending_operation()?;
        if self.immediate_exit()? {
            return Ok(0);
        }
        self.vcpu.clear_run_output()?;

        let mut guest_mode =
            GuestMode::new(&self.vcpu.guest_context, &self.vcpu.lapic, &self.vcpu.lapic);
        let mut consecutive_preemption_timer_exits = 0_u64;

        loop {
            let eptp = self.vm.guest_mem().eptp();
            let exit_info = match guest_mode.execute(eptp as u64) {
                Ok(exit_info) => exit_info,
                Err(err) => {
                    error!("rustshyper: GuestMode::execute failed: {:?}", err);
                    return Err(err.into());
                }
            };
            let exit_info = match VmxExitReason::try_from(exit_info.exit_reason) {
                Ok(VmxExitReason::IO_INSTRUCTION) => Some(exit_info),
                Ok(VmxExitReason::EPT_VIOLATION) => {
                    let handled =
                        emulate_apic_mmio(self.vcpu.clone(), exit_info.guest_phys_addr as u64)
                            .inspect_err(|err| {
                                error!(
                                    "rustshyper: APIC MMIO handling failed: reason={:#x}, len={}, \
                             rip={:#x}, gpa={:#x}, qualification={:#x}, err={:?}",
                                    exit_info.exit_reason,
                                    exit_info.instruction_len,
                                    exit_info.guest_rip,
                                    exit_info.guest_phys_addr,
                                    exit_info.exit_qualification,
                                    err
                                );
                            })?;
                    if handled {
                        consecutive_preemption_timer_exits = 0;
                        None
                    } else {
                        Some(exit_info)
                    }
                }
                Ok(VmxExitReason::PREEMPTION_TIMER) => {
                    consecutive_preemption_timer_exits =
                        consecutive_preemption_timer_exits.saturating_add(1);
                    if consecutive_preemption_timer_exits >= PREEMPTION_TIMER_USER_EXIT_INTERVAL {
                        Some(exit_info)
                    } else {
                        None
                    }
                }
                Ok(VmxExitReason::HLT) => {
                    consecutive_preemption_timer_exits = 0;
                    self.vcpu
                        .guest_context()
                        .advance_rip(exit_info.instruction_len as _);
                    loop {
                        if self.vcpu.wait_for_hlt_wakeup() {
                            break None;
                        }

                        // TODO: Use a more efficient wait mechanism instead of busy-waiting.
                        Task::yield_now();
                    }
                }
                Ok(VmxExitReason::PAUSE_INSTRUCTION) => {
                    consecutive_preemption_timer_exits = 0;
                    None
                }
                Ok(_) => Some(exit_info),
                Err(_) => Some(exit_info),
            };

            if let Some(exit_info) = exit_info {
                self.write_exit_to_run_page(exit_info)?;
                return Ok(0);
            }
        }
    }

    fn complete_pending_operation(&self) -> Result<()> {
        let Some(operation) = self.vcpu.take_pending_operation() else {
            return Ok(());
        };

        if let Err(err) = self.complete_operation(operation) {
            self.vcpu.set_pending_operation(operation);
            return Err(err);
        }
        Ok(())
    }

    fn complete_operation(&self, operation: PendingOperation) -> Result<()> {
        match operation {
            PendingOperation::Pio(pio) => self.complete_pio_operation(pio),
            PendingOperation::Mmio(mmio) => self.complete_mmio_operation(mmio),
        }
    }

    fn complete_pio_operation(&self, operation: PendingPioOperation) -> Result<()> {
        if operation.direction == PioDirection::In {
            let mut bytes = [0_u8; size_of::<u64>()];
            let size = operation.size as usize;
            self.vcpu
                .read_run_bytes(KVM_RUN_IO_DATA_OFFSET, &mut bytes[..size])?;
            let value = u64::from_le_bytes(bytes);
            self.vcpu
                .guest_context()
                .set_gpr(GPR_RAX, operation.size, value);
        }

        self.vcpu
            .guest_context()
            .advance_rip(u64::from(operation.instruction_len));
        Ok(())
    }

    fn complete_mmio_operation(&self, operation: PendingMmioOperation) -> Result<()> {
        if !operation.is_write {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "MMIO read completion needs instruction decoding"
            );
        }

        self.vcpu
            .guest_context()
            .advance_rip(u64::from(operation.instruction_len));
        Ok(())
    }

    fn immediate_exit(&self) -> Result<bool> {
        let immediate_exit = self
            .vcpu
            .read_run_val::<u8>(KVM_RUN_IMMEDIATE_EXIT_OFFSET)?;
        Ok(immediate_exit != 0)
    }

    fn write_exit_to_run_page(&self, exit_info: GuestExitInfo) -> Result<()> {
        self.vcpu.clear_run_output()?;
        self.write_common_run_state()?;

        match VmxExitReason::try_from(exit_info.exit_reason) {
            Ok(VmxExitReason::IO_INSTRUCTION) => self.write_io_exit(exit_info),
            Ok(VmxExitReason::EPT_VIOLATION) => self.write_mmio_exit(exit_info),
            Ok(VmxExitReason::HLT) => self.write_simple_exit(KVM_EXIT_HLT),
            Ok(VmxExitReason::PREEMPTION_TIMER) => self.write_simple_exit(KVM_EXIT_INTR),
            _ => self.write_internal_error_exit(exit_info),
        }
    }

    fn write_common_run_state(&self) -> Result<()> {
        let sregs = self.vcpu.get_sregs()?;
        self.vcpu
            .write_run_val(KVM_RUN_READY_FOR_INTERRUPT_INJECTION_OFFSET, &0_u8)?;
        self.vcpu.write_run_val(KVM_RUN_IF_FLAG_OFFSET, &0_u8)?;
        self.vcpu.write_run_val(KVM_RUN_FLAGS_OFFSET, &0_u16)?;
        self.vcpu.write_run_val(KVM_RUN_CR8_OFFSET, &sregs.cr8)?;
        self.vcpu
            .write_run_val(KVM_RUN_APIC_BASE_OFFSET, &sregs.apic_base)?;
        Ok(())
    }

    fn write_simple_exit(&self, exit_reason: u32) -> Result<()> {
        self.vcpu
            .write_run_val(KVM_RUN_EXIT_REASON_OFFSET, &exit_reason)
    }

    fn write_io_exit(&self, exit_info: GuestExitInfo) -> Result<()> {
        let qualification = exit_info.exit_qualification;
        if qualification & ((1 << 4) | (1 << 5)) != 0 {
            return self.write_internal_error_exit(exit_info);
        }

        let size = ((qualification & 0b111) as u8).saturating_add(1);
        if !matches!(size, 1 | 2 | 4) {
            return self.write_internal_error_exit(exit_info);
        }

        let direction = if qualification & (1 << 3) != 0 {
            PioDirection::In
        } else {
            PioDirection::Out
        };
        let kvm_direction = match direction {
            PioDirection::In => KVM_EXIT_IO_IN,
            PioDirection::Out => KVM_EXIT_IO_OUT,
        };
        let port = ((qualification >> 16) & 0xffff) as u16;
        let count = 1_u32;
        let data_offset = KVM_RUN_IO_DATA_OFFSET as u64;

        self.write_simple_exit(KVM_EXIT_IO)?;
        self.vcpu
            .write_run_val(KVM_RUN_IO_DIRECTION_OFFSET, &kvm_direction)?;
        self.vcpu.write_run_val(KVM_RUN_IO_SIZE_OFFSET, &size)?;
        self.vcpu.write_run_val(KVM_RUN_IO_PORT_OFFSET, &port)?;
        self.vcpu.write_run_val(KVM_RUN_IO_COUNT_OFFSET, &count)?;
        self.vcpu
            .write_run_val(KVM_RUN_IO_DATA_OFFSET_OFFSET, &data_offset)?;

        if direction == PioDirection::Out {
            let rax = self.vcpu.guest_context().gpr(GPR_RAX);
            let bytes = rax.to_le_bytes();
            self.vcpu
                .write_run_bytes(KVM_RUN_IO_DATA_OFFSET, &bytes[..size as usize])?;
        }

        self.vcpu
            .set_pending_operation(PendingOperation::Pio(PendingPioOperation {
                direction,
                size,
                instruction_len: exit_info.instruction_len,
            }));
        Ok(())
    }

    fn write_mmio_exit(&self, exit_info: GuestExitInfo) -> Result<()> {
        let is_write = exit_info.exit_qualification & (1 << 1) != 0;
        let is_write_byte = u8::from(is_write);
        let len = 0_u32;

        self.write_simple_exit(KVM_EXIT_MMIO)?;
        self.vcpu.write_run_val(
            KVM_RUN_MMIO_PHYS_ADDR_OFFSET,
            &(exit_info.guest_phys_addr as u64),
        )?;
        self.vcpu
            .write_run_bytes(KVM_RUN_MMIO_DATA_OFFSET, &[0_u8; 8])?;
        self.vcpu.write_run_val(KVM_RUN_MMIO_LEN_OFFSET, &len)?;
        self.vcpu
            .write_run_val(KVM_RUN_MMIO_IS_WRITE_OFFSET, &is_write_byte)?;

        if is_write {
            self.vcpu
                .set_pending_operation(PendingOperation::Mmio(PendingMmioOperation {
                    is_write,
                    instruction_len: exit_info.instruction_len,
                }));
        }
        Ok(())
    }

    fn write_internal_error_exit(&self, exit_info: GuestExitInfo) -> Result<()> {
        warn!(
            "rustshyper: unsupported VM exit for KVM_RUN: reason={:#x}, len={}, rip={:#x}, \
             gpa={:#x}, qualification={:#x}",
            exit_info.exit_reason,
            exit_info.instruction_len,
            exit_info.guest_rip,
            exit_info.guest_phys_addr,
            exit_info.exit_qualification,
        );
        self.write_simple_exit(KVM_EXIT_INTERNAL_ERROR)
    }
}

impl crate::process::signal::Pollable for VcpuFile {
    fn poll(
        &self,
        _mask: crate::events::IoEvents,
        _poller: Option<&mut crate::process::signal::PollHandle>,
    ) -> crate::events::IoEvents {
        // VCPUs don't support polling
        crate::events::IoEvents::empty()
    }
}
