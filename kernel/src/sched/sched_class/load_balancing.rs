// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};
use core::sync::atomic::Ordering::Relaxed;

use ostd::{
    cpu::{all_cpus, CpuId},
    sync::SpinLock,
};

use super::{ClassScheduler, PerCpuClassRqSet, SchedClassRq, UpdateLoadFlags};
use crate::thread::AsThread;

struct SchedDomain {
    parent: usize,
    // TODO: Replace indirect references by CpuId with static direct references.
    cpus: Vec<CpuId>,
}

impl SchedDomain {
    fn rqs<'c>(
        &'c self,
        sched: &'c ClassScheduler,
        except: CpuId,
    ) -> impl Iterator<Item = &'c SpinLock<PerCpuClassRqSet>> + 'c {
        (self.cpus.iter())
            .filter_map(move |cpu| (cpu != &except).then(|| &sched.rqs[cpu.as_usize()]))
    }
}

pub(super) struct SchedDomains(Vec<SchedDomain>);

impl SchedDomains {
    pub fn all() -> Self {
        // TODO: Initialize all domains according to the actual CPU topology.
        Self(vec![SchedDomain {
            parent: usize::MAX,
            cpus: all_cpus().collect(),
        }])
    }

    fn ancestors(&self, mut domain_id: usize) -> impl Iterator<Item = &SchedDomain> + '_ {
        core::iter::from_fn(move || {
            let domain = self.0.get(domain_id)?;
            domain_id = domain.parent;
            Some(domain)
        })
    }
}

impl PerCpuClassRqSet {
    fn load_balance_rq(&mut self, busiest: &mut PerCpuClassRqSet, mut delta: u64) -> usize {
        let now_ns = super::sched_clock();

        let mut items = busiest.fair.migrate_start();
        let mut cursor = items.len() - 1;
        let mut count = 0;

        while delta > 0 {
            let thread = items[cursor].0 .0.as_thread().unwrap();
            let load = thread.sched_attr().fair_load.data().total_weight;

            if let Some(new_delta) = delta.checked_sub(load)
                && thread.atomic_cpu_affinity().contains(self.cpu, Relaxed)
            {
                delta = new_delta;
                count += 1;

                busiest.update_thread_load(now_ns, Some(&thread), Some(UpdateLoadFlags::Detach));

                ostd::early_println!(
                    "Moving {:p} from {:?} to {:?}",
                    thread,
                    busiest.cpu,
                    self.cpu
                );
                let task = items.remove(cursor).0 .0;
                let thread = task.as_thread().unwrap().clone();
                self.enqueue_entity((task, thread), None);
            }

            cursor = match cursor.checked_sub(1) {
                Some(new_cursor) => new_cursor,
                None => break,
            };
        }

        busiest.fair.migrate_end(items);
        count
    }

    pub fn load_balance(&mut self) -> usize {
        let sched = &super::CLASS_SCHEDULER;

        let now_ns = super::sched_clock();
        self.update_rq_load(now_ns, true);

        let mut count = 0;

        for domain in sched.domains.ancestors(self.leaf_domain) {
            let now_ns = super::sched_clock();

            let Some((mut busiest, max)) = domain.rqs(sched, self.cpu).fold(None, |busiest, rq| {
                let Some(mut rq) = rq.try_lock() else {
                    return busiest;
                };
                rq.update_rq_load(now_ns, true);

                // TODO: Add considerations for non-FAIR loads.
                let load = rq.fair_load.data().total_weight;
                match busiest {
                    None => Some((rq, load)),
                    Some((_, busiest_load)) if load > busiest_load => Some((rq, load)),
                    _ => busiest,
                }
            }) else {
                continue;
            };

            if busiest.fair.is_empty() {
                continue;
            }

            let delta = max.checked_sub(self.fair_load.data().total_weight);
            let Some(delta) = delta else {
                continue;
            };

            // We want to average the load, not transferring the delta.
            let delta = (delta + 1) / 2;

            count += self.load_balance_rq(&mut busiest, delta);
        }

        count
    }
}
