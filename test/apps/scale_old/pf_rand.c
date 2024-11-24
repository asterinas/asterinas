#include "common.h"

#define NUM_PAGES 1024 // Number of pages to allocate per thread for mmap

void *worker_thread(void *arg)
{
	thread_data_t *data = (thread_data_t *)arg;

	long tsc_start, tsc_end;

	// Wait for the main thread to signal that all threads are ready
	while (__atomic_load_n(&DISPATCH_LIGHT, __ATOMIC_ACQUIRE) == 0) {
		sched_yield();
	}

	tsc_start = rdtsc();

	// unmap them one by one randomly
	int *page_idx = data->page_idx + data->thread_id * NUM_PAGES;
	for (size_t i = 0; i < NUM_PAGES; i++) {
		char *region = data->region + page_idx[i] * PAGE_SIZE;
		// Trigger page fault
		region[0] = 1;
	}

	tsc_end = rdtsc();
	long tot_time = get_time_in_nanos(tsc_start, tsc_end);

	data->lat = tot_time / NUM_PAGES;

	return NULL;
}

int main(int argc, char *argv[])
{
	return entry_point(argc, argv, worker_thread,
			   (test_config_t){ .num_prealloc_pages_per_thread =
						    NUM_PAGES,
					    .num_prealloc_pages = 0,
					    .trigger_fault_before_spawn = 0,
					    .rand_assign_pages = 1 });
}
