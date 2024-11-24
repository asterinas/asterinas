#include "common.h"

#define NUM_PAGES 16384 // Number of pages to allocate totally for mmap

void *worker_thread(void *arg)
{
	thread_start();

	size_t num_pages_per_thread = NUM_PAGES / data->tot_threads;

	// Trigger page fault one by one
	for (size_t i = 0; i < num_pages_per_thread; i++) {
		data->base[(data->thread_id * num_pages_per_thread + i) *
			   PAGE_SIZE] = 1;
		request_end();
	}

	// Barrier at here. Output pagetable size and vma tree size.

	return NULL;
}

int main(int argc, char *argv[])
{
	printf("***MEM_USAGE_SIM***\n");

	run_test_specify_threads(384, worker_thread,
				 (test_config_t){ .num_total_pages = NUM_PAGES,
						  .num_requests_per_thread = 0,
						  .num_pages_per_request = 0,
						  .num_pages_pad = 0,
						  .mmap_before_spawn = 0,
						  .trigger_fault_before_spawn =
							  0,
						  .contention_level = 0,
						  .is_unfixed_mmap_test = 0 });
	printf("\n");
}
