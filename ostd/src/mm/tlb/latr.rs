// SPDX-License-Identifier: MPL-2.0

//! LATR TLB flushing.

use core::{
    cell::{RefCell, SyncUnsafeCell},
    sync::atomic::{AtomicBool, Ordering},
};

use super::{OpsStack, TlbFlushOp};
use crate::{
    cpu::{all_cpus, num_cpus, AtomicCpuSet, CpuId, CpuSet, PinCurrentCpu},
    cpu_local, cpu_local_cell,
    mm::{frame::meta::AnyFrameMeta, Frame},
    task::disable_preempt,
    trap::irq::{self, DisabledLocalIrqGuard},
};

pub(crate) fn init_bsp() {
    for cpu in all_cpus() {
        SHARED_ARRAY.get_on_cpu(cpu).call_once(|| {
            core::array::from_fn(|_| {
                (
                    AtomicCpuSet::new(CpuSet::new_empty()),
                    SyncUnsafeCell::new(None),
                )
            })
        });
    }

    crate::timer::register_callback(timer_flush);
}

pub(crate) fn init_this_cpu() {
    crate::timer::register_callback(timer_flush);
}

fn timer_flush() {
    cpu_local_cell! {
        static COUNTER: usize = 0;
    }

    let cur_cpu = CpuId::current_racy();
    let count = COUNTER.load();

    if count % num_cpus() == cur_cpu.as_usize() {
        // Do less flushes.
        do_flush();
    }

    COUNTER.add_assign(1);
}

pub(crate) fn do_flush() {
    let preempt_guard = disable_preempt();
    let cur_cpu = preempt_guard.current_cpu();

    for cpu in all_cpus() {
        if cpu == cur_cpu {
            continue;
        }
        if !HAVE_FRAME_FOR_OTHERS
            .get_on_cpu(cpu)
            .load(Ordering::Relaxed)
        {
            continue;
        }

        let shared_arr = SHARED_ARRAY.get_on_cpu(cpu).get().unwrap();

        let mut ops = super::OpsStack::new();
        for (set, op) in shared_arr.iter() {
            if set.contains(cur_cpu, Ordering::Relaxed) {
                core::sync::atomic::fence(Ordering::Acquire);
                // SAFETY: It is read only before we clear our ID from the set.
                let op = unsafe { &*op.get() };
                if let Some(op) = op {
                    ops.push(op.clone(), None);
                }
                set.remove(cur_cpu, Ordering::Release);
            }
        }

        ops.flush_all();
    }
}

#[expect(dead_code)]
pub(crate) fn do_recycle() {
    let mut to_be_dropped = [const { Option::<Frame<dyn AnyFrameMeta>>::None }; LATR_ARRAY_SIZE];
    let mut have_frame_for_others = false;
    {
        let irq_guard = irq::disable_local();
        let cur_cpu = irq_guard.current_cpu();

        if HAVE_FRAME_FOR_OTHERS
            .get_on_cpu(cur_cpu)
            .load(Ordering::Relaxed)
        {
            return;
        }

        let shared_arr = SHARED_ARRAY.get_on_cpu(cur_cpu).get().unwrap();
        let frames_arr = FRAMES.get_with(&irq_guard);
        let mut frames = frames_arr.borrow_mut();

        for ((slot, (set, _)), to_drop) in frames
            .iter_mut()
            .zip(shared_arr.iter())
            .zip(to_be_dropped.iter_mut())
        {
            if slot.is_none() {
                continue;
            }

            if set.load(Ordering::Relaxed).is_empty() {
                *to_drop = slot.take();
                continue;
            } else {
                have_frame_for_others = true;
            }
        }

        if !have_frame_for_others {
            HAVE_FRAME_FOR_OTHERS
                .get_on_cpu(cur_cpu)
                .store(false, Ordering::Relaxed);
        }
    }
    // Drop it after enabling IRQs.
    drop(to_be_dropped);
}

pub(super) fn add_lazy_frame(
    cpu_set: &CpuSet,
    op: TlbFlushOp,
    frame: Frame<dyn AnyFrameMeta>,
) -> core::result::Result<(), Frame<dyn AnyFrameMeta>> {
    let mut to_be_dropped = None;
    let mut frame = Some(frame);

    {
        let irq_guard = irq::disable_local();

        let gather = CURRENT_GATHER.get_with(&irq_guard);
        let mut gather = gather.borrow_mut();
        gather.push(op.clone(), None);

        let cur_cpu = irq_guard.current_cpu();

        let shared_arr = SHARED_ARRAY.get_on_cpu(cur_cpu).get().unwrap();
        let frames_arr = FRAMES.get_with(&irq_guard);
        let mut frames = frames_arr.borrow_mut();

        let last_checked_idx = LAST_CHECKED_IDX.load();
        for i in 0..LATR_ARRAY_SIZE {
            let idx = (last_checked_idx + i) % LATR_ARRAY_SIZE;

            let slot = &mut frames[idx];
            let (set_ref, op_ref) = &shared_arr[idx];

            if !slot.is_none() && !set_ref.load(Ordering::Relaxed).is_empty() {
                continue;
            }

            if slot.is_none() {
                debug_assert!(set_ref.load(Ordering::Relaxed).is_empty());
            }

            // SAFETY: The CPU set is currently empty so no one will read it.
            *unsafe { &mut *op_ref.get() } = Some(op);
            core::sync::atomic::fence(Ordering::Release);
            set_ref.store(cpu_set, Ordering::Relaxed);

            to_be_dropped = slot.take();
            *slot = frame.take();

            LAST_CHECKED_IDX.store((idx + 1) % LATR_ARRAY_SIZE);
            break;
        }

        HAVE_FRAME_FOR_OTHERS
            .get_on_cpu(cur_cpu)
            .store(true, Ordering::Relaxed);
    }

    // Drop it after enabling IRQs.
    drop(to_be_dropped);

    if let Some(frame) = frame {
        Err(frame)
    } else {
        Ok(())
    }
}

pub(super) fn flush_local_gather(irq_guard: &DisabledLocalIrqGuard) {
    let gather = CURRENT_GATHER.get_with(irq_guard);
    let mut gather = gather.borrow_mut();
    gather.flush_all();
}

const LATR_ARRAY_SIZE: usize = 64;

cpu_local! {
    static SHARED_ARRAY: spin::Once<[(AtomicCpuSet, SyncUnsafeCell<Option<TlbFlushOp>>); LATR_ARRAY_SIZE]> = spin::Once::new();
    static FRAMES: RefCell<[Option<Frame<dyn AnyFrameMeta>>; LATR_ARRAY_SIZE]> = RefCell::new([const { None }; LATR_ARRAY_SIZE]);

    static HAVE_FRAME_FOR_OTHERS: AtomicBool = AtomicBool::new(false);

    static CURRENT_GATHER: RefCell<OpsStack> = RefCell::new(OpsStack::new());
}

cpu_local_cell! {
    static LAST_CHECKED_IDX: usize = 0;
}
