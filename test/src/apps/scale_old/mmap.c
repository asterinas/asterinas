#include "common.h"

#define NUM_MMAPS 512 // Number mmaps per thread
#define NUM_PAGES (((NUM_MMAPS) * sizeof(char *) + (PAGE_SIZE)-1) / (PAGE_SIZE))

void *worker_thread(void *arg)
{
	thread_data_t *data = (thread_data_t *)arg;

	long tsc_start, tsc_end;

	// Wait for the main thread to signal that all threads are ready
	while (__atomic_load_n(&DISPATCH_LIGHT, __ATOMIC_ACQUIRE) == 0) {
		sched_yield();
	}

	tsc_start = rdtsc();

	// map them one by one, use preallocated region to store the pointers
	char **region = (char **)(data->region +
				  data->thread_id * NUM_PAGES * PAGE_SIZE);
	for (size_t i = 0; i < NUM_MMAPS; i++) {
		region[i] = mmap(NULL, PAGE_SIZE * 8, PROT_READ | PROT_WRITE,
				 MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	}

	tsc_end = rdtsc();
	long tot_time = get_time_in_nanos(tsc_start, tsc_end);

	data->lat = tot_time / NUM_MMAPS;

	for (size_t i = 0; i < NUM_MMAPS; i++) {
		munmap(region[i], PAGE_SIZE * 8);
	}

	return NULL;
}

int main(int argc, char *argv[])
{
	return entry_point(argc, argv, worker_thread,
			   (test_config_t){ .num_prealloc_pages_per_thread =
						    NUM_PAGES,
					    .num_prealloc_pages = 0,
					    .trigger_fault_before_spawn = 1,
					    .rand_assign_pages = 0 });
}