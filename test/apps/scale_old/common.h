#define _GNU_SOURCE
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <pthread.h>
#include <unistd.h>
#include <string.h>
#include <time.h>

long rdtsc(void)
{
	unsigned int hi, lo;
	__asm__ __volatile__("rdtsc" : "=a"(lo), "=d"(hi));
	return ((long)lo) | (((long)hi) << 32);
}

long get_time_in_nanos(long start_tsc, long end_tsc)
{
	// Our setup is a 1.9 GHz CPU
	return (end_tsc - start_tsc) * 10 / 19;
}

unsigned int simple_get_rand(unsigned int last_rand)
{
	return ((long)last_rand * 1103515245 + 12345) & 0x7fffffff;
}

size_t up_align(size_t size, size_t alignment)
{
	return ((size) + ((alignment)-1)) & ~((alignment)-1);
}

#define PAGE_SIZE 4096 // Typical page size in bytes
// Single thread test, will be executed 4096 times;
// 8 thread test will be executed 512 times;
// 128 thread test will be executed 32 times, etc.
// Statistics are per-thread basis.
#define TOT_THREAD_RUNS 4096

#define RESULT_FILE "results"

int DISPATCH_LIGHT;

typedef struct {
	char *region;
	// For random tests
	int *page_idx;
	size_t region_size;
	int thread_id;
	int tot_threads;
	// Pass the result back to the main thread
	long lat;
} thread_data_t;

typedef struct {
	size_t num_prealloc_pages_per_thread;
	size_t num_prealloc_pages;
	int trigger_fault_before_spawn;
	int rand_assign_pages;
} test_config_t;

// Decls

int entry_point(int argc, char *argv[], void *(*worker_thread)(void *),
		test_config_t config);
void run_test_specify_threads(int num_threads, void *(*worker_thread)(void *),
			      test_config_t config);
void run_test_specify_rounds(int num_threads, void *(*worker_thread)(void *),
			     test_config_t config);
void run_test_forked(int num_threads, void *(*worker_thread)(void *),
		     test_config_t config);
void run_test(int num_threads, void *(*worker_thread)(void *),
	      test_config_t config);

// Impls

int entry_point(int argc, char *argv[], void *(*worker_thread)(void *),
		test_config_t config)
{
	int num_threads;
	if (argc == 1) {
		num_threads = -1;
	} else if (argc == 2) {
		num_threads = atoi(argv[1]);
	} else {
		fprintf(stderr, "Usage: %s [num_threads]\n", argv[0]);
		exit(EXIT_FAILURE);
	}

	run_test_specify_threads(num_threads, worker_thread, config);

	return 0;
}

void run_test_specify_threads(int num_threads, void *(*worker_thread)(void *),
			      test_config_t config)
{
	// Get the number of CPUs via sched_getaffinity
	cpu_set_t cpuset;
	if (sched_getaffinity(0, sizeof(cpu_set_t), &cpuset) != 0) {
		perror("sched_getaffinity failed");
		exit(EXIT_FAILURE);
	}
	int num_cpus = CPU_COUNT(&cpuset);

	printf("Threads, p5 lat (ns), Avg lat (ns), p95 lat (ns), Pos err lat (ns2), Neg err lat (ns2)\n");

	if (num_threads == -1) {
		int threads[] = { 1, 2, 4, 8, 16, 32, 48, 64, 80, 96, 112, 128 };
		for (int i = 0; i < sizeof(threads) / sizeof(int); i++) {
			if (threads[i] > num_cpus)
				break;
			run_test_specify_rounds(threads[i], worker_thread,
						config);
		}
	} else {
		run_test_specify_rounds(num_threads, worker_thread, config);
	}
}

pthread_t threads[TOT_THREAD_RUNS];
thread_data_t thread_data[TOT_THREAD_RUNS];
long thread_lat[TOT_THREAD_RUNS];

void run_test_specify_rounds(int num_threads, void *(*worker_thread)(void *),
			     test_config_t config)
{
	remove(RESULT_FILE);

	int runs = TOT_THREAD_RUNS / num_threads;
	for (int run_id = 0; run_id < runs; run_id++) {
		run_test_forked(num_threads, worker_thread, config);
	}

	// Read latency data from RESULT_FILE
	FILE *file = fopen(RESULT_FILE, "r");
	if (file == NULL) {
		perror("fopen failed");
		exit(EXIT_FAILURE);
	}
	int tot_runs = 0;
	long lat = 0;
	while (fscanf(file, "%ld", &lat) == 1) {
		thread_lat[tot_runs++] = lat;
	}
	fclose(file);
	if (tot_runs != num_threads * runs) {
		perror("Incorrect number of runs");
		exit(EXIT_FAILURE);
	}

	// Calculate the p5, average, p95, and variance of the latencies
	long avg = 0;
	for (int i = 0; i < tot_runs; i++) {
		avg += thread_lat[i];
	}
	avg /= tot_runs;
	long posvar2 = 0;
	long numpos = 0;
	long negvar2 = 0;
	long numneg = 0;
	for (int i = 0; i < tot_runs; i++) {
		long diff = thread_lat[i] - avg;
		if (diff > 0) {
			posvar2 += diff * diff;
			numpos++;
		} else {
			negvar2 += diff * diff;
			numneg++;
		}
	}
	posvar2 /= numpos;
	negvar2 /= numneg;

	// Calculate the p5 and p95 latencies
	// bubble sort
	for (int i = 0; i < tot_runs; i++) {
		for (int j = i + 1; j < tot_runs; j++) {
			if (thread_lat[i] > thread_lat[j]) {
				long temp = thread_lat[i];
				thread_lat[i] = thread_lat[j];
				thread_lat[j] = temp;
			}
		}
	}
	long p5 = thread_lat[tot_runs / 20];
	long p95 = thread_lat[tot_runs * 19 / 20];

	printf("%d, %ld, %ld, %ld, %ld, %ld\n", num_threads, p5, avg, p95,
	       posvar2, negvar2);
}

void run_test_forked(int num_threads, void *(*worker_thread)(void *),
		     test_config_t config)
{
	// Spawn a process for a test in order to avoid interference between tests
	int pid = fork();
	if (pid == -1) {
		perror("fork failed");
		exit(EXIT_FAILURE);
	} else if (pid == 0) {
		// Child process
		run_test(num_threads, worker_thread, config);
		exit(EXIT_SUCCESS);
	} else {
		// Parent process
		wait(NULL);
	}
}

void run_test(int num_threads, void *(*worker_thread)(void *),
	      test_config_t config)
{
	size_t num_tot_pages =
		config.num_prealloc_pages_per_thread * num_threads +
		config.num_prealloc_pages;
	int trigger_fault_before_spawn = config.trigger_fault_before_spawn;
	int rand_assign_pages = config.rand_assign_pages;

	char *region = mmap(NULL, num_tot_pages * PAGE_SIZE,
			    PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS,
			    -1, 0);

	if (region == MAP_FAILED) {
		perror("mmap failed");
		exit(EXIT_FAILURE);
	}

	if (trigger_fault_before_spawn) {
		// Trigger page faults before spawning threads
		for (int i = 0; i < num_tot_pages; i++) {
			region[i * PAGE_SIZE] = 1;
		}
	}

	int *page_idx = NULL;
	int page_idx_size = 0;
	if (rand_assign_pages) {
		page_idx_size =
			up_align(num_tot_pages * sizeof(int), PAGE_SIZE);
		page_idx = mmap(NULL, page_idx_size, PROT_READ | PROT_WRITE,
				MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
		for (int i = 0; i < num_tot_pages; i++) {
			page_idx[i] = i;
		}

		// Random shuffle
		unsigned int rand = 0xdeadbeef - num_threads;
		for (int i = num_tot_pages - 1; i > 0; i--) {
			rand = simple_get_rand(rand);
			int j = rand % (i + 1);
			int temp = page_idx[i];
			page_idx[i] = page_idx[j];
			page_idx[j] = temp;
		}
	}

	// Initialize global variables
	__atomic_clear(&DISPATCH_LIGHT, __ATOMIC_RELEASE);

	// Create threads and trigger page faults in parallel
	for (int i = 0; i < num_threads; i++) {
		thread_data[i].region = region;
		thread_data[i].region_size = num_tot_pages * PAGE_SIZE;
		thread_data[i].page_idx = page_idx;
		thread_data[i].thread_id = i;
		thread_data[i].tot_threads = num_threads;

		if (pthread_create(&threads[i], NULL, worker_thread,
				   &thread_data[i]) != 0) {
			perror("pthread_create failed");
			exit(EXIT_FAILURE);
		}

		// Set the thread affinity to a specific core
		cpu_set_t cpuset;
		CPU_ZERO(&cpuset);
		CPU_SET(i, &cpuset);
		if (pthread_setaffinity_np(threads[i], sizeof(cpu_set_t),
					   &cpuset) != 0) {
			perror("pthread_setaffinity_np failed");
			exit(EXIT_FAILURE);
		}
	}

	// Signal all threads to start
	__atomic_store_n(&DISPATCH_LIGHT, 1, __ATOMIC_RELEASE);

	// Join threads
	for (int i = 0; i < num_threads; i++) {
		pthread_join(threads[i], NULL);
	}

	munmap(region, num_tot_pages * PAGE_SIZE);
	if (rand_assign_pages) {
		munmap(page_idx, page_idx_size);
	}

	// Write latency data to RESULT_FILE
	FILE *file = fopen(RESULT_FILE, "a");
	if (file == NULL) {
		perror("fopen failed");
		exit(EXIT_FAILURE);
	}
	for (int i = 0; i < num_threads; i++) {
		fprintf(file, "%ld\n", thread_data[i].lat);
	}
	fclose(file);
}
