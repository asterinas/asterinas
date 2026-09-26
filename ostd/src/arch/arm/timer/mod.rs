// SPDX-License-Identifier: MPL-2.0

//! The timer support.

use core::{
    arch::asm,
    sync::atomic::{AtomicUsize, Ordering},
};

use spin::Once;

use crate::{
    arch::{
        boot::DEVICE_TREE,
        irq::{IRQ_CHIP, InterruptSourceInFdt, MappedIrqLine},
        read_tsc,
        trap::TrapFrame,
        tsc_freq,
    },
    cpu_local_cell,
    irq::IrqLine,
    timer::TIMER_FREQ,
};

static TIMER_IRQ: Once<MappedIrqLine> = Once::new();

static COUNTER_STEP: AtomicUsize = AtomicUsize::new(0);
cpu_local_cell! {
    static COUNTER_VAL: usize = 0;
}

pub(super) fn init() {
    if let Err(err) = init_impl() {
        crate::error!("Failed to initialize timer, error: {:?}", err);
    }
}

#[derive(Debug)]
enum InitError {
    NotPresent,
    NoInterruptParent,
    InvalidInterrupts,
    IrqLineAlloc,
    IrqLineMap,
}

fn init_impl() -> Result<(), InitError> {
    // Reference: <https://www.kernel.org/doc/Documentation/devicetree/bindings/timer/arm%2Carch_timer.txt>
    const FDT_COMPATIBLE: &[&str; 2] = &["arm,armv8-timer", "arm,armv7-timer"];

    let device_tree = DEVICE_TREE.get().unwrap();

    let timer = device_tree
        .find_compatible(FDT_COMPATIBLE)
        .ok_or(InitError::NotPresent)?;

    // FIXME: We need to find the "interrupt-parent" property for the nearest ancestor. However,
    // there are no APIs to iterate ancestors. This workaround uses the "interrupt-parent" property
    // of the root node.
    let intr_parent = if let Some(root) = device_tree.find_node("/")
        && let Some(parent) = root.property("interrupt-parent")
        && let Some(intr_parent) = parent.as_usize()
    {
        intr_parent as u32
    } else {
        return Err(InitError::NoInterruptParent);
    };
    let intr_args = if let Some(intrs) = timer.property("interrupts")
        && let mut iter = intrs
            .value
            .as_chunks::<{ size_of::<u32>() }>()
            .0
            .iter()
            .map(|chunk| u32::from_be_bytes(*chunk))
        // "Interrupt list for secure, non-secure, virtual and hypervisor timers, in that order."
        && let Ok(_secure) = iter.next_chunk::<3>()
        && let Ok(_non_secure) = iter.next_chunk::<3>()
        && let Ok(_virtual) = iter.next_chunk::<3>()
        && let Ok(_hypervisor) = iter.next_chunk::<3>()
    {
        _virtual
    } else {
        return Err(InitError::NoInterruptParent);
    };

    let irq_line = IrqLine::alloc().map_err(|_| InitError::IrqLineAlloc)?;
    let mut mapped_irq_line = IRQ_CHIP
        .get()
        .unwrap()
        .map_fdt_pin_to(
            InterruptSourceInFdt {
                interrupt_parent: intr_parent,
                arguments: intr_args,
            },
            irq_line,
        )
        .map_err(|_| InitError::IrqLineMap)?;
    mapped_irq_line.on_active(timer_callback);
    TIMER_IRQ.call_once(|| mapped_irq_line);

    COUNTER_STEP.store((tsc_freq() / TIMER_FREQ) as usize, Ordering::Relaxed);
    COUNTER_VAL.store(read_tsc() as usize);

    // SAFETY: It is safe to enable timer interrupts.
    unsafe {
        asm!(
            // IMASK, bit [1]:   0 - Timer interrupt is not masked.
            // ENABLE, bit [1]:  1 - Timer enabled.
            "mov {tmp}, #1",
            "msr cntv_ctl_el0, {tmp}",
            tmp = out(reg) _,
        );
    }

    set_next_timer();

    Ok(())
}

fn timer_callback(trapframe: &TrapFrame) {
    crate::timer::call_timer_callback_functions(trapframe);

    set_next_timer();
}

fn set_next_timer() {
    let new_val = {
        let old_val = COUNTER_VAL.load();
        let step = COUNTER_STEP.load(Ordering::Relaxed);
        old_val.wrapping_add(step)
    };

    COUNTER_VAL.store(new_val);

    // SAFETY: It is safe to update the timer compare value.
    unsafe {
        asm!(
            "msr cntv_cval_el0, {new_val}",
            new_val = in(reg) new_val,
        );
    }
}
