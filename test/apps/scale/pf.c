#include "common.h"

#define NUM_PAGES 32 // Number of pages to allocate per thread for mmap
#define NUM_PAGES_PAD (512 - NUM_PAGES)

void *worker_thread(void *arg)
{
	thread_start();

	// Trigger page fault one by one
	for (size_t i = 0; i < NUM_PAGES; i++) {
		data->base[data->offset[i]] = 1;
		request_end();
	}

	return NULL;
}

int main(int argc, char *argv[])
{
	if (argc > 3 || argc < 2) {
		fprintf(stderr, "Usage: %s contention_level [num_threads]\n",
			argv[0]);
		exit(EXIT_FAILURE);
	}

	int num_threads = -1;
	int contention_level = atoi(argv[1]);
	if (argc == 3) {
		num_threads = read_num_threads(argv[2]);
	} else {
		num_threads = -1;
	}

	printf("***PF %s***\n", contention_level_name[contention_level]);
	run_test_specify_threads(
		num_threads, worker_thread,
		(test_config_t){ .num_requests_per_thread = NUM_PAGES,
				 .num_pages_per_request = 1,
				 .num_pages_pad = NUM_PAGES_PAD,
				 .mmap_before_spawn = 1,
				 .trigger_fault_before_spawn = 0,
				 .contention_level = contention_level,
				 .is_unfixed_mmap_test = 0 });
	printf("\n");
}
