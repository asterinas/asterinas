#include "common.h"

#define NUM_PAGES 16384 // Number of pages to allocate totally for mmap

void *worker_thread(void *arg)
{
	thread_data_t *data = (thread_data_t *)arg;

	long tsc_start, tsc_end;

	// Wait for the main thread to signal that all threads are ready
	while (__atomic_load_n(&DISPATCH_LIGHT, __ATOMIC_ACQUIRE) == 0) {
		sched_yield();
	}

	tsc_start = rdtsc();

	size_t per_thread_size = data->region_size / data->tot_threads;
	char *region = data->region + data->thread_id * per_thread_size;
	for (size_t i = 0; i < per_thread_size / PAGE_SIZE; i++) {
		region[i * PAGE_SIZE] = 1; // Trigger page fault
	}

	tsc_end = rdtsc();
	long tot_time = get_time_in_nanos(tsc_start, tsc_end);

	data->lat = tot_time / (per_thread_size / PAGE_SIZE);

	return NULL;
}

int main(int argc, char *argv[])
{
	return entry_point(argc, argv, worker_thread,
			   (test_config_t){ .num_prealloc_pages_per_thread = 0,
					    .num_prealloc_pages = NUM_PAGES,
					    .trigger_fault_before_spawn = 0,
					    .rand_assign_pages = 0 });
}
