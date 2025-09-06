use aster_util::per_cpu_counter::PerCpuCounter;
use spin::Once;

/// The total number of fork, vfork and clone.
pub(super) static FORKS_COUNTER: Once<PerCpuCounter> = Once::new();

/// Returns the total number of fork, vfork and clone.
pub fn forks_count() -> usize {
    FORKS_COUNTER.get().unwrap().sum_all_cpus()
}
