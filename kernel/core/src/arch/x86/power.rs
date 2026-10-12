// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use ostd::{
    arch::{
        device::io_port::{ReadWriteAccess, WriteOnlyAccess},
        kernel::{ACPI_INFO, AcpiPmInfo},
    },
    io::IoPort,
    power::ExitCode,
};
use spin::Once;

use crate::process::signal::constants::SIGPWR;

static ACPI_RESET_PORT_AND_VAL: Once<(IoPort<u8, WriteOnlyAccess>, u8)> = Once::new();

/// PM1 status registers (a, optional b) and control registers, for the power
/// button and for S5.
struct Pm1 {
    info: AcpiPmInfo,
    status_a: IoPort<u16, ReadWriteAccess>,
    status_b: Option<IoPort<u16, ReadWriteAccess>>,
    /// Kept so the port stays acquired (the enable bit must stay set).
    _enable_a: IoPort<u16, ReadWriteAccess>,
    control_a: IoPort<u16, ReadWriteAccess>,
    control_b: Option<IoPort<u16, ReadWriteAccess>>,
}

static PM1: Once<Pm1> = Once::new();

/// PM1_STS.PWRBTN_STS: the power button was pressed.
const PM1_STS_PWRBTN: u16 = 1 << 8;
/// PM1_EN.PWRBTN_EN. QEMU only latches `PWRBTN_STS` while this is set; the
/// SCI it then raises stays masked at the IOAPIC (nothing unmasks GSI 9),
/// so enabling the event costs nothing and we poll the status bit instead.
const PM1_EN_PWRBTN: u16 = 1 << 8;
/// PM1_CNT: SLP_TYP field and SLP_EN bit.
const PM1_CNT_SLP_TYP_SHIFT: u16 = 10;
const PM1_CNT_SLP_TYP_MASK: u16 = 0x7 << PM1_CNT_SLP_TYP_SHIFT;
const PM1_CNT_SLP_EN: u16 = 1 << 13;

/// Enters S5 (soft off) through the fixed hardware registers.
///
/// Needs the `_S5` sleep type, which `ostd` extracts from the DSDT/SSDTs
/// without an AML interpreter. Does not return if the firmware honours it;
/// otherwise the lower-priority handlers (a restart) follow.
///
/// S5 is what `aws ec2 stop-instances` waits for: the instance leaves
/// `stopping` as soon as the guest is off, instead of after EC2's
/// multi-minute hard stop.
fn try_acpi_s5(_code: ExitCode) {
    // If possible, keep this method panic-free because it may be called by the panic handler.
    let Some(pm) = PM1.get() else { return };
    let Some((typ_a, typ_b)) = pm.info.s5_sleep_type else {
        return;
    };
    let set = |port: &IoPort<u16, ReadWriteAccess>, typ: u8| {
        let cur = port.read() & !PM1_CNT_SLP_TYP_MASK & !PM1_CNT_SLP_EN;
        let val = cur | ((typ as u16) << PM1_CNT_SLP_TYP_SHIFT);
        port.write(val);
        port.write(val | PM1_CNT_SLP_EN);
    };
    set(&pm.control_a, typ_a);
    if let Some(b) = &pm.control_b {
        set(b, typ_b);
    }
    // The write is the last thing a powering-off machine does; give the
    // chipset a moment before we conclude it did not work.
    for _ in 0..1_000_000 {
        core::hint::spin_loop();
    }
}
crate::register_poweroff_handler!(try_acpi_s5, crate::power::Priority::FIRMWARE);

/// Polls PM1_STS for a power button press and delivers `SIGPWR` to init.
///
/// Called from the timer tick. EC2 `stop-instances`, `reboot-instances` and
/// `terminate-instances` all press the ACPI power button; without this the
/// request stalls until the hypervisor hard-resets after several minutes.
/// Polling (rather than the SCI) keeps interrupt routing out of the picture;
/// the status bit latches whether or not the SCI is enabled.
fn poll_power_button() {
    static TICKS: AtomicU32 = AtomicU32::new(0);
    static LAST_PRESS_TICK: AtomicU32 = AtomicU32::new(0);
    const POLL_EVERY: u32 = 50; // ticks: 50 ms at TIMER_FREQ = 1000
    const DEBOUNCE: u32 = 5_000; // one SIGPWR per 5 s at most

    let tick = TICKS.fetch_add(1, Ordering::Relaxed);
    if !tick.is_multiple_of(POLL_EVERY) {
        return;
    }
    let Some(pm) = PM1.get() else { return };
    let mut pressed = false;
    if pm.status_a.read() & PM1_STS_PWRBTN != 0 {
        pm.status_a.write(PM1_STS_PWRBTN); // write-1-to-clear
        pressed = true;
    }
    if let Some(b) = &pm.status_b
        && b.read() & PM1_STS_PWRBTN != 0
    {
        b.write(PM1_STS_PWRBTN);
        pressed = true;
    }
    if !pressed {
        return;
    }
    let last = LAST_PRESS_TICK.load(Ordering::Relaxed);
    if last != 0 && tick.wrapping_sub(last) < DEBOUNCE {
        return;
    }
    LAST_PRESS_TICK.store(tick.max(1), Ordering::Relaxed);
    ostd::early_println!("[kernel] acpi: power button pressed, sending SIGPWR to init");
    if let Some(init) = crate::init::init_process_weak() {
        crate::process::enqueue_signal_async(init, SIGPWR);
    }
}

fn init_pm1(info: AcpiPmInfo) {
    let acquire = |port: u16| IoPort::<u16, ReadWriteAccess>::acquire(port).ok();
    let enable_port = info.pm1a_event_port + (info.pm1_event_len / 2) as u16;
    let (Some(status_a), Some(enable_a), Some(control_a)) = (
        acquire(info.pm1a_event_port),
        acquire(enable_port),
        acquire(info.pm1a_control_port),
    ) else {
        ostd::warn!("ACPI PM1 ports are not available");
        return;
    };
    // Drop a press that happened before we were listening, then enable the event.
    status_a.write(PM1_STS_PWRBTN);
    enable_a.write(enable_a.read() | PM1_EN_PWRBTN);
    let status_b = info.pm1b_event_port.and_then(acquire);
    let control_b = info.pm1b_control_port.and_then(acquire);
    PM1.call_once(|| Pm1 {
        info,
        status_a,
        status_b,
        _enable_a: enable_a,
        control_a,
        control_b,
    });
    ostd::early_println!(
        "[kernel] acpi: PM1a event {:#x} control {:#x}, S5 sleep type {:?}; power button -> SIGPWR",
        info.pm1a_event_port,
        info.pm1a_control_port,
        info.s5_sleep_type
    );
    ostd::timer::register_callback_on_cpu(poll_power_button);
}

fn try_acpi_reset(_code: ExitCode) {
    // If possible, keep this method panic-free because it may be called by the panic handler.
    if let Some((port, val)) = ACPI_RESET_PORT_AND_VAL.get() {
        port.write(*val);
    }
}
// ACPI is attempted before legacy restart fallbacks, following Linux's x86 reset order.
// Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/reboot.c#L657>
crate::register_restart_handler!(try_acpi_reset, crate::power::Priority::DEFAULT);

crate::register_poweroff_handler!(
    ostd::arch::power::try_poweroff,
    crate::power::Priority::HIGH
);

// The triple fault comes after the i8042 controller (which registers at `Priority::new(1)`), as
// the last resort. It is the only method that works on machines without an ACPI reset register
// or a keyboard controller, such as EC2 Nitro instances.
crate::register_restart_handler!(ostd::arch::power::try_restart, crate::power::Priority::LOW);

/// Powers off by restarting.
///
/// Without an ACPI AML interpreter the kernel cannot enter S5 on real hardware or on EC2, so a
/// guest whose init process asked for `poweroff` would otherwise hang with a dead console. A
/// restart re-runs the same image, which is what an auto-scaling group wants from a crashed node.
fn try_poweroff_by_restart(code: ExitCode) {
    ostd::power::restart(code);
}
crate::register_poweroff_handler!(try_poweroff_by_restart, crate::power::Priority::LOW);

pub(super) fn init() {
    let acpi_info = ACPI_INFO.get().unwrap();

    if let Some((reset_port_num, reset_val)) = acpi_info.reset_port_and_val {
        if let Ok(reset_port) = IoPort::acquire(reset_port_num) {
            ACPI_RESET_PORT_AND_VAL.call_once(move || (reset_port, reset_val));
        } else {
            ostd::warn!("The reset port from ACPI is not available");
        }
    }

    if let Some(pm) = acpi_info.pm {
        init_pm1(pm);
    }
}
