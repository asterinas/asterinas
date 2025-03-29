// SPDX-License-Identifier: MPL-2.0

//! BRAVO readers-writer lock.

use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use super::pfq::PfqRwLock;
use crate::cpu::CpuId;

#[derive(Debug)]
#[repr(C)]
pub struct BravoPfqRwLock {
    inhibit_until_and_rbias: AtomicU32,
    underlying: PfqRwLock,
}

#[derive(Debug)]
pub struct BravoReadGuard {
    pos: Option<usize>,
}

const BRAVO_HASH_TABLE_SIZE: usize = 1021;
static VISIBLE_READERS: [AtomicPtr<BravoPfqRwLock>; BRAVO_HASH_TABLE_SIZE] =
    [const { AtomicPtr::new(core::ptr::null_mut()) }; BRAVO_HASH_TABLE_SIZE];

impl BravoPfqRwLock {
    const INHIBIT_MULTIPLIER: u32 = 9;

    pub const fn new() -> Self {
        Self {
            underlying: PfqRwLock::new(),
            inhibit_until_and_rbias: AtomicU32::new(0),
        }
    }

    pub fn lock_read(&self) -> BravoReadGuard {
        if self.rbias() {
            let pos = self.hash();
            let self_ptr = self as *const Self as *mut Self;
            if VISIBLE_READERS[pos]
                .compare_exchange(
                    core::ptr::null_mut(),
                    self_ptr,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                if self.rbias() {
                    return BravoReadGuard { pos: Some(pos) };
                } else {
                    VISIBLE_READERS[pos].store(core::ptr::null_mut(), Ordering::Relaxed);
                }
            }
        }
        self.underlying.lock_read();
        if !self.rbias() && self.cur_time() >= self.inhibit_until() {
            self.store_rbias(true);
        }
        BravoReadGuard { pos: None }
    }

    pub fn unlock_read(&self, rguard: BravoReadGuard) {
        if let Some(pos) = rguard.pos {
            let self_ptr = self as *const Self as *mut Self;
            assert_eq!(VISIBLE_READERS[pos].load(Ordering::Relaxed), self_ptr);
            VISIBLE_READERS[pos].store(core::ptr::null_mut(), Ordering::Relaxed);
        } else {
            self.underlying.unlock_read();
        }
    }

    pub fn lock_write(&self) {
        self.underlying.lock_write();
        if self.rbias() {
            self.store_rbias(false);
            let start_time = self.cur_time();
            let self_ptr = self as *const Self as *mut Self;
            for reader in VISIBLE_READERS.iter() {
                while reader.load(Ordering::SeqCst) == self_ptr {
                    core::hint::spin_loop();
                }
            }
            let end_time = self.cur_time();
            let inhibit_time =
                start_time.checked_sub(end_time).unwrap_or(1_000_000) * Self::INHIBIT_MULTIPLIER;
            self.store_inhibit_until(end_time.wrapping_add(inhibit_time));
        }
    }

    pub fn unlock_write(&self) {
        self.underlying.unlock_write();
    }

    fn rbias(&self) -> bool {
        self.inhibit_until_and_rbias.load(Ordering::Relaxed) & 1 != 0
    }

    fn store_rbias(&self, rbias: bool) {
        if rbias {
            self.inhibit_until_and_rbias.fetch_or(1, Ordering::Relaxed);
        } else {
            self.inhibit_until_and_rbias
                .fetch_and(!1, Ordering::Relaxed);
        }
    }

    fn inhibit_until(&self) -> u32 {
        self.inhibit_until_and_rbias.load(Ordering::Relaxed) >> 1
    }

    fn store_inhibit_until(&self, inhibit_until: u32) {
        self.inhibit_until_and_rbias
            .fetch_or(inhibit_until << 1, Ordering::Relaxed);
    }

    fn cur_time(&self) -> u32 {
        (crate::arch::read_tsc() >> 3) as u32
    }

    fn hash(&self) -> usize {
        let a = CpuId::current_racy().as_usize();
        let b = a.wrapping_add(self as *const Self as usize);
        let c = (a << 16).wrapping_add(b >> 16);
        c % BRAVO_HASH_TABLE_SIZE
    }
}
