// SPDX-License-Identifier: MPL-2.0
use alloc::{sync::Arc, vec::Vec};

use ostd::{cpu::PinCurrentCpu, timer::Jiffies};
use spin::Once;

use crate::{sched::SchedPolicy, thread::Thread, time::clocks::CpuClock};

#[derive(Debug, Clone, Copy)]
pub struct Cpustat {
    pub user: Jiffies,
    pub nice: Jiffies,
    pub system: Jiffies,
    pub idle: Jiffies,
    pub iowait: Jiffies,
    pub irq: Jiffies,
    pub softirq: Jiffies,
    pub steal: Jiffies,
    pub guest: Jiffies,
    pub guest_nice: Jiffies,
}

struct _Cpustat {
    user: Arc<CpuClock>,
    nice: Arc<CpuClock>,
    system: Arc<CpuClock>,
    idle: Arc<CpuClock>,
    iowait: Arc<CpuClock>,
    irq: Arc<CpuClock>,
    softirq: Arc<CpuClock>,
    steal: Arc<CpuClock>,
    guest: Arc<CpuClock>,
    guest_nice: Arc<CpuClock>,
}

impl _Cpustat {
    fn new() -> Self {
        Self {
            user: CpuClock::new(),
            nice: CpuClock::new(),
            system: CpuClock::new(),
            idle: CpuClock::new(),
            iowait: CpuClock::new(),
            irq: CpuClock::new(),
            softirq: CpuClock::new(),
            steal: CpuClock::new(),
            guest: CpuClock::new(),
            guest_nice: CpuClock::new(),
        }
    }

    // read all, return a snapshot
    fn load(&self) -> Cpustat {
        Cpustat {
            user: self.user.read_jiffies(),
            nice: self.nice.read_jiffies(),
            system: self.system.read_jiffies(),
            idle: self.idle.read_jiffies(),
            iowait: self.iowait.read_jiffies(),
            irq: self.irq.read_jiffies(),
            softirq: self.softirq.read_jiffies(),
            steal: self.steal.read_jiffies(),
            guest: self.guest.read_jiffies(),
            guest_nice: self.guest_nice.read_jiffies(),
        }
    }
}

pub struct CpuStatManager {
    per_cpu_stats: Vec<_Cpustat>,
    global_stats: _Cpustat,
}

impl CpuStatManager {
    pub fn get() -> &'static Arc<CpuStatManager> {
        static INSTANCE: Once<Arc<CpuStatManager>> = Once::new();
        INSTANCE.call_once(|| {
            let num_cpus = ostd::cpu::num_cpus();
            Arc::new(CpuStatManager::new(num_cpus))
        })
    }

    pub fn get_on_cpu(&self, cpu: usize) -> Cpustat {
        self.per_cpu_stats[cpu].load()
    }

    pub fn get_global(&self) -> Cpustat {
        self.global_stats.load()
    }

    pub fn inc_user_time(&self, cpu: usize, val: u64) {
        if cpu < self.per_cpu_stats.len() {
            self.per_cpu_stats[cpu].user.add_jiffies(val);
            self.global_stats.user.add_jiffies(val);
        }
    }
    pub fn inc_system_time(&self, cpu: usize, val: u64) {
        if cpu < self.per_cpu_stats.len() {
            self.per_cpu_stats[cpu].system.add_jiffies(val);
            self.global_stats.system.add_jiffies(val);
        }
    }
    pub fn inc_idle_time(&self, cpu: usize, val: u64) {
        if cpu < self.per_cpu_stats.len() {
            self.per_cpu_stats[cpu].idle.add_jiffies(val);
            self.global_stats.idle.add_jiffies(val);
        }
    }

    fn new(num_cpus: usize) -> Self {
        let mut per_cpu_stats = Vec::with_capacity(num_cpus);
        for _ in 0..num_cpus {
            per_cpu_stats.push(_Cpustat::new());
        }
        CpuStatManager {
            per_cpu_stats,
            global_stats: _Cpustat::new(),
        }
    }
}

pub fn cpu_stat_manager() -> &'static Arc<CpuStatManager> {
    CpuStatManager::get()
}
// callback at timer irq
fn update_cpu_statistics() {
    let _guard = ostd::task::disable_preempt();
    let manager = CpuStatManager::get();
    let cpu_id = _guard.current_cpu().as_usize();
    let is_kernel = ostd::arch::trap::is_kernel_interrupted();

    if is_idle() {
        manager.inc_idle_time(cpu_id, 1);
        return; // idle time is not counted towards CPU usage
    }
    if is_kernel {
        manager.inc_system_time(cpu_id, 1);
    } else {
        manager.inc_user_time(cpu_id, 1);
    }
}

fn is_idle() -> bool {
    if let Some(current_thread) = Thread::current() {
        current_thread.sched_attr().policy() == SchedPolicy::Idle
    } else {
        false
    }
}

pub fn init() {
    ostd::timer::register_callback(update_cpu_statistics);
}
