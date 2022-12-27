use crate::{cell::Cell, debug, x86_64_util};
use lazy_static::lazy_static;
use volatile::{
    access::{ReadOnly, ReadWrite, WriteOnly},
    Volatile,
};

pub(crate) const IA32_APIC_BASE_MSR: u32 = 0x1B;
pub(crate) const IA32_APIC_BASE_MSR_BSP: u32 = 0x100; // Processor is a BSP
pub(crate) const IA32_APIC_BASE_MSR_ENABLE: u32 = 0x800;

const APIC_LVT_MASK_BITS: u32 = 1 << 16;

lazy_static! {
    pub static ref APIC_INSTANCE: Cell<APIC> = Cell::new(APIC::new());
}

#[derive(Debug)]
pub struct APIC {
    local_apic_id_register: Volatile<&'static mut u32, ReadWrite>,
    local_apic_version_register: Volatile<&'static u32, ReadOnly>,

    task_priority_register: Volatile<&'static mut u32, ReadWrite>,
    arbitration_priority_register: Volatile<&'static u32, ReadOnly>,
    processor_priority_register: Volatile<&'static u32, ReadOnly>,
    pub eoi_register: Volatile<&'static mut u32, WriteOnly>,
    remote_read_register: Volatile<&'static u32, ReadOnly>,
    logical_destination_register: Volatile<&'static mut u32, ReadWrite>,
    destination_format_register: Volatile<&'static mut u32, ReadWrite>,
    spurious_interrupt_vector_register: Volatile<&'static mut u32, ReadWrite>,

    /// total 256 bits, 32 bits per element
    isr_per_32_bits: [Volatile<&'static u32, ReadOnly>; 8],

    /// total 256 bits, 32 bits per element
    tmr_per_32_bits: [Volatile<&'static u32, ReadOnly>; 8],

    /// total 256 bits, 32 bits per element
    irr_per_32_bits: [Volatile<&'static u32, ReadOnly>; 8],

    pub error_status_register: Volatile<&'static u32, ReadOnly>,

    lvt_cmci_register: Volatile<&'static mut u32, ReadWrite>,
    icr_bits_31_0: Volatile<&'static mut u32, ReadWrite>,
    icr_bits_63_32: Volatile<&'static mut u32, ReadWrite>,
    pub lvt_timer_register: Volatile<&'static mut u32, ReadWrite>,
    lvt_thermal_sensor_register: Volatile<&'static mut u32, ReadWrite>,
    lvt_performance_monitoring_counters_register: Volatile<&'static mut u32, ReadWrite>,
    lvt_lint0_register: Volatile<&'static mut u32, ReadWrite>,
    lvt_lint1_register: Volatile<&'static mut u32, ReadWrite>,
    lvt_error_register: Volatile<&'static mut u32, ReadWrite>,
    pub initial_count_register: Volatile<&'static mut u32, ReadWrite>,
    pub current_count_register: Volatile<&'static u32, ReadOnly>,

    pub divide_configuration_register: Volatile<&'static mut u32, ReadWrite>,
}

impl APIC {
    pub fn new() -> Self {
        let base_address = get_apic_base_address();

        let local_apic_id_register = Self::new_read_write_volatile(base_address + 0x0020);
        let local_apic_version_register = Self::new_read_only_volatile(base_address + 0x0030);

        let task_priority_register = Self::new_read_write_volatile(base_address + 0x0080);
        let arbitration_priority_register = Self::new_read_only_volatile(base_address + 0x0090);
        let processor_priority_register = Self::new_read_only_volatile(base_address + 0x00A0);
        let eoi_register = Self::new_write_only_volatile(base_address + 0x00B0);
        let remote_read_register = Self::new_read_only_volatile(base_address + 0x00C0);
        let logical_destination_register = Self::new_read_write_volatile(base_address + 0x00D0);
        let destination_format_register = Self::new_read_write_volatile(base_address + 0x00E0);
        let spurious_interrupt_vector_register =
            Self::new_read_write_volatile(base_address + 0x00F0);

        let mut isr_per_32_bits: [Volatile<&'static u32, ReadOnly>; 8] =
            unsafe { core::mem::MaybeUninit::uninit().assume_init() };
        for i in 0..8 {
            isr_per_32_bits[i] = Self::new_read_only_volatile(base_address + 0x0100 + i * 0x0010);
        }

        let mut tmr_per_32_bits: [Volatile<&'static u32, ReadOnly>; 8] =
            unsafe { core::mem::MaybeUninit::uninit().assume_init() };
        for i in 0..8 {
            tmr_per_32_bits[i] = Self::new_read_only_volatile(base_address + 0x0180 + i * 0x0010);
        }

        let mut irr_per_32_bits: [Volatile<&'static u32, ReadOnly>; 8] =
            unsafe { core::mem::MaybeUninit::uninit().assume_init() };
        for i in 0..8 {
            irr_per_32_bits[i] = Self::new_read_only_volatile(base_address + 0x0200 + i * 0x0010);
        }

        let error_status_register = Self::new_read_only_volatile(base_address + 0x0280);

        let lvt_cmci_register = Self::new_read_write_volatile(base_address + 0x02F0);
        let icr_bits_31_0 = Self::new_read_write_volatile(base_address + 0x0300);
        let icr_bits_63_32 = Self::new_read_write_volatile(base_address + 0x0310);
        let lvt_timer_register = Self::new_read_write_volatile(base_address + 0x0320);
        let lvt_thermal_sensor_register = Self::new_read_write_volatile(base_address + 0x0330);
        let lvt_performance_monitoring_counters_register =
            Self::new_read_write_volatile(base_address + 0x0340);
        let lvt_lint0_register = Self::new_read_write_volatile(base_address + 0x0350);
        let lvt_lint1_register = Self::new_read_write_volatile(base_address + 0x0360);
        let lvt_error_register = Self::new_read_write_volatile(base_address + 0x0370);
        let initial_count_register = Self::new_read_write_volatile(base_address + 0x0380);
        let current_count_register = Self::new_read_only_volatile(base_address + 0x0390);

        let divide_configuration_register = Self::new_read_write_volatile(base_address + 0x03E0);

        Self {
            local_apic_id_register,
            local_apic_version_register,
            task_priority_register,
            arbitration_priority_register,
            processor_priority_register,
            eoi_register,
            remote_read_register,
            logical_destination_register,
            destination_format_register,
            spurious_interrupt_vector_register,
            isr_per_32_bits,
            tmr_per_32_bits,
            irr_per_32_bits,
            error_status_register,
            lvt_cmci_register,
            icr_bits_31_0,
            icr_bits_63_32,
            lvt_timer_register,
            lvt_thermal_sensor_register,
            lvt_performance_monitoring_counters_register,
            lvt_lint0_register,
            lvt_lint1_register,
            lvt_error_register,
            initial_count_register,
            current_count_register,
            divide_configuration_register,
        }
    }
    #[inline(always)]
    fn new_read_only_volatile(pa: usize) -> Volatile<&'static u32, ReadOnly> {
        Volatile::new_read_only(Self::convert_pa_to_u32_ref(pa))
    }
    #[inline(always)]
    fn new_read_write_volatile(pa: usize) -> Volatile<&'static mut u32, ReadWrite> {
        Volatile::new(Self::convert_pa_to_u32_ref(pa))
    }
    #[inline(always)]
    fn new_write_only_volatile(pa: usize) -> Volatile<&'static mut u32, WriteOnly> {
        Volatile::new_write_only(Self::convert_pa_to_u32_ref(pa))
    }

    #[inline(always)]
    fn convert_pa_to_u32_ref(pa: usize) -> &'static mut u32 {
        unsafe { &mut *(crate::mm::address::phys_to_virt(pa) as *mut usize as *mut u32) }
    }
}

pub(crate) fn has_apic() -> bool {
    let value = unsafe { x86_64_util::cpuid(1) };
    value.edx & 0x100 != 0
}

pub(crate) fn init() {
    super::pic::disable_temp();

    let apic_lock = APIC_INSTANCE.get();
    // enable apic
    set_apic_base_address(get_apic_base_address());
    let spurious = apic_lock.spurious_interrupt_vector_register.read();
    apic_lock
        .spurious_interrupt_vector_register
        .write(spurious | (0x100));
    let apic_id = apic_lock.local_apic_id_register.read() >> 24;
    let apic_ver = apic_lock.local_apic_version_register.read();

    debug!(
        "APIC ID:{:x}, Version:{:x}, Max LVT:{:x}",
        apic_id,
        apic_ver & 0xff,
        (apic_ver >> 16) & 0xff
    );

    debug!(
        "LDR:{:x}, DFR:{:x}",
        apic_lock.logical_destination_register.read(),
        apic_lock.destination_format_register.read()
    );
    debug!("spurious:{:x}", spurious);

    drop(apic_lock);
}

#[inline(always)]
pub fn ack() {
    let lock = APIC_INSTANCE.get();
    lock.eoi_register.write(0);
}

/// set APIC base address and enable it
fn set_apic_base_address(address: usize) {
    x86_64_util::set_msr(
        IA32_APIC_BASE_MSR,
        address | IA32_APIC_BASE_MSR_ENABLE as usize,
    )
}

/// get APIC base address
fn get_apic_base_address() -> usize {
    x86_64_util::get_msr(IA32_APIC_BASE_MSR) & 0xf_ffff_f000
}
