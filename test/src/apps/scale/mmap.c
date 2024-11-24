#include "common.h"

#define NUM_MMAPS 32 // Number mmaps per thread
#define PAGES_PER_MMAP 4
#define NUM_PAGES_PAD (512 - NUM_MMAPS * PAGES_PER_MMAP)

void *worker_thread(void *arg)
{
	thread_start();

	if (data->is_unfixed_mmap_test) {
		// map them one by one
		for (size_t i = 0; i < NUM_MMAPS; i++) {
			mmap(NULL, PAGE_SIZE * PAGES_PER_MMAP,
			     PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
			request_end();
		}
	} else {
		// map them one by one
		for (size_t i = 0; i < NUM_MMAPS; i++) {
			mmap(data->base + data->offset[i],
			     PAGE_SIZE * PAGES_PER_MMAP, PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_FIXED | MAP_ANONYMOUS, -1, 0);
			request_end();
		}
	}

	return NULL;
}

int main(int argc, char *argv[])
{
	if (argc > 4) {
		fprintf(stderr,
			"Usage: %s ((fixed contention_level)|unfixed) [num_threads]\n",
			argv[0]);
		exit(EXIT_FAILURE);
	}

	int is_fixed = 0;
	int contention_level = 0;
	int num_threads = -1;
	if (strcmp(argv[1], "unfixed") == 0) {
		is_fixed = 0;
		if (argc == 3) {
			num_threads = read_num_threads(argv[2]);
		}
	} else if (strcmp(argv[1], "fixed") == 0) {
		is_fixed = 1;
		if (argc < 3) {
			fprintf(stderr, "Contention level is required\n");
			exit(EXIT_FAILURE);
		}
		contention_level = atoi(argv[2]);
		if (contention_level < 0 || contention_level > 2) {
			fprintf(stderr, "Invalid contention level: %s\n",
				argv[2]);
			exit(EXIT_FAILURE);
		}
		if (argc == 4) {
			num_threads = read_num_threads(argv[3]);
		}
	} else {
		fprintf(stderr, "Invalid argument for fixed/unfixed: %s\n",
			argv[1]);
		exit(EXIT_FAILURE);
	}

	if (is_fixed) {
		printf("***MMAP FIXED %s***\n",
		       contention_level_name[contention_level]);
		run_test_specify_threads(
			num_threads, worker_thread,
			(test_config_t){ .num_requests_per_thread = NUM_MMAPS,
					 .num_pages_per_request =
						 PAGES_PER_MMAP,
					 .num_pages_pad = NUM_PAGES_PAD,
					 .mmap_before_spawn = 0,
					 .trigger_fault_before_spawn = 0,
					 .contention_level = contention_level,
					 .is_unfixed_mmap_test = 0 });
		printf("\n");
	} else {
		printf("***MMAP UNFIXED***\n");
		run_test_specify_threads(
			num_threads, worker_thread,
			(test_config_t){ .num_requests_per_thread = NUM_MMAPS,
					 .num_pages_per_request =
						 PAGES_PER_MMAP,
					 .num_pages_pad = 0,
					 .mmap_before_spawn = 0,
					 .trigger_fault_before_spawn = 0,
					 .contention_level = 0,
					 .is_unfixed_mmap_test = 1 });
		printf("\n");
	}
}