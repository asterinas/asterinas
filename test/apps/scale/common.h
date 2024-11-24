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
	// Our setup is 2_250_000_000 Hz
	return (end_tsc - start_tsc) * 100 / 225;
}

unsigned int simple_get_rand(unsigned int last_rand)
{
	return ((long)last_rand * 1103515245 + 12345) & 0x7fffffff;
}

size_t up_align(size_t size, size_t alignment)
{
	return ((size) + ((alignment)-1)) & ~((alignment)-1);
}

int read_num_threads(char *arg)
{
	int num_threads = atoi(arg);
	if (num_threads <= 0) {
		fprintf(stderr,
			"Invalid number of threads: %s. Please provide a positive integer greater than 0.\n",
			arg);
		exit(EXIT_FAILURE);
	}
	return num_threads;
}

#define PAGE_SIZE 4096 // Typical page size in bytes

// Single thread test, will be executed 4096 times;
// 8 thread test will be executed 512 times;
// 128 thread test will be executed 32 times, etc.
// Statistics are per-thread basis.
#define TOT_THREAD_RUNS 4096

#define MAX_REQUESTS_PER_THREAD 64

char *const BASE_PTR = (char *)0x100000000UL;

#define RESULT_FILE "results"

#define thread_start()                                                     \
	int cur_request = 0;                                               \
	thread_data_t *data = (thread_data_t *)arg;                        \
	long tsc_last, tsc_cur;                                            \
	/* Wait for the main thread to signal that all threads are ready*/ \
	while (__atomic_load_n(&DISPATCH_LIGHT, __ATOMIC_ACQUIRE) == 0) {  \
		sched_yield();                                             \
	}                                                                  \
	tsc_last = rdtsc();

#define request_end()                                     \
	tsc_cur = rdtsc();                                \
	long time = get_time_in_nanos(tsc_last, tsc_cur); \
	data->lat[cur_request++] = time;                  \
	tsc_last = tsc_cur;

int DISPATCH_LIGHT;

typedef struct {
	char *base;
	long *offset;
	int thread_id;
	int tot_threads;
	int is_unfixed_mmap_test;

	// Pass the latency(ns) result back to the main thread
	long lat[MAX_REQUESTS_PER_THREAD];
} thread_data_t;

typedef struct {
	// Only for mem usage tests
	size_t num_total_pages;

	// Only for time usage tests
	size_t num_requests_per_thread;
	size_t num_pages_per_request;
	size_t num_pages_pad;
	int mmap_before_spawn;
	int trigger_fault_before_spawn;
	int contention_level;
	int is_unfixed_mmap_test;
} test_config_t;

const char *contention_level_name[] = { "LOW_CONTENTION", "HIGH_CONTENTION" };

// Decls

void run_test_specify_threads(int num_threads, void *(*worker_thread)(void *),
			      test_config_t config);
void run_test_and_print(int num_threads, void *(*worker_thread)(void *),
			test_config_t config);
void run_test_forked(int num_threads, void *(*worker_thread)(void *),
		     test_config_t config);
void run_test(int num_threads, void *(*worker_thread)(void *),
	      test_config_t config);

// Impls

void run_test_specify_threads(int num_threads, void *(*worker_thread)(void *),
			      test_config_t config)
{
	// Add safety check to prevent buffer overflow
	if (config.num_requests_per_thread > MAX_REQUESTS_PER_THREAD) {
		fprintf(stderr,
			"Error: num_requests_per_thread (%zu) exceeds maximum (%d)\n",
			config.num_requests_per_thread,
			MAX_REQUESTS_PER_THREAD);
		exit(EXIT_FAILURE);
	}

	// Gets the number of CPUS via sched_affinity
	size_t cpuset_size = CPU_ALLOC_SIZE(CPU_SETSIZE);
	cpu_set_t *cpuset = CPU_ALLOC(CPU_SETSIZE);
	if (cpuset == NULL) {
		perror("CPU_ALLOC failed");
		exit(EXIT_FAILURE);
	}
	CPU_ZERO_S(cpuset_size, cpuset);
	if (sched_getaffinity(0, cpuset_size, cpuset) != 0) {
		perror("sched_getaffinity failed");
		CPU_FREE(cpuset);
		exit(EXIT_FAILURE);
	}
	int num_cpus = CPU_COUNT_S(cpuset_size, cpuset);
	CPU_FREE(cpuset);

	if (num_threads == -1) {
		int threads[] = { 1,  2,   4,	8,   16,  32,
				  64, 128, 192, 256, 320, 384 };
		for (int i = 0; i < sizeof(threads) / sizeof(int); i++) {
			if (threads[i] > num_cpus)
				break;
			run_test_and_print(threads[i], worker_thread, config);
		}
	} else {
		run_test_and_print(num_threads, worker_thread, config);
	}
}

pthread_t threads[TOT_THREAD_RUNS];
thread_data_t thread_data[TOT_THREAD_RUNS];
long thread_lat[TOT_THREAD_RUNS][MAX_REQUESTS_PER_THREAD];

void run_test_and_print(int num_threads, void *(*worker_thread)(void *),
			test_config_t config)
{
	remove(RESULT_FILE);

	int runs = TOT_THREAD_RUNS / num_threads;
	for (int run_id = 0; run_id < runs; run_id++) {
		run_test_forked(num_threads, worker_thread, config);
	}
	int tot_nr_results = runs * num_threads;

	// Read latencies data from RESULT_FILE
	FILE *file = fopen(RESULT_FILE, "r");
	if (file == NULL) {
		perror("fopen failed");
		exit(EXIT_FAILURE);
	}

	for (int i = 0; i < tot_nr_results; i++) {
		size_t num_requests_this_thread;
		if (fscanf(file, "%ld", &num_requests_this_thread) != 1) {
			fprintf(stderr, "Incorrect number of runs\n");
			exit(EXIT_FAILURE);
		}
		if (num_requests_this_thread !=
		    config.num_requests_per_thread) {
			fprintf(stderr,
				"Incorrect number of requests for thread %d: "
				"%ld, expected %ld\n",
				i, num_requests_this_thread,
				config.num_requests_per_thread);
		}
		for (int j = 0; j < config.num_requests_per_thread; j++) {
			if (fscanf(file, " %ld", &thread_lat[i][j]) != 1) {
				perror("Incorrect number of runs");
				exit(EXIT_FAILURE);
			}
		}
	}

	printf("<#)<+< RESULTS of %d threads >+>(#>\n", num_threads);

	// Calculate average latency
	long avg_lat[MAX_REQUESTS_PER_THREAD] = { 0 };

	for (int j = 0; j < config.num_requests_per_thread; j++) {
		for (int i = 0; i < tot_nr_results; i++) {
			avg_lat[j] += thread_lat[i][j];
		}
		avg_lat[j] /= tot_nr_results;
	}

	printf(" Avg Lat (ns):");
	for (int j = 0; j < config.num_requests_per_thread; j++) {
		printf(" %ld", avg_lat[j]);
	}
	printf("\n");
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
	      test_config_t cfg)
{
	char *base = NULL;
	long *offset = NULL;

	if (cfg.num_total_pages != 0) {
		// Memory usage tests
		base = mmap(NULL, cfg.num_total_pages * PAGE_SIZE,
			    PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS,
			    -1, 0);
		if (base == MAP_FAILED) {
			perror("mmap failed");
			exit(EXIT_FAILURE);
		}
	} else {
		// Time usage tests
		size_t num_tot_requests =
			cfg.num_requests_per_thread * num_threads;

		size_t offset_size =
			up_align(num_tot_requests * sizeof(long), PAGE_SIZE);
		offset = mmap(NULL, offset_size, PROT_READ | PROT_WRITE,
			      MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
		if (offset == MAP_FAILED) {
			perror("mmap failed");
			exit(EXIT_FAILURE);
		}

		unsigned long reserved_region_size =
			(cfg.num_pages_per_request + cfg.num_pages_pad) *
			PAGE_SIZE;

		for (int i = 0; i < num_threads; i++) {
			for (int j = 0; j < cfg.num_requests_per_thread; j++) {
				offset[i * cfg.num_requests_per_thread + j] =
					i * reserved_region_size +
					j * cfg.num_pages_per_request *
						PAGE_SIZE;
			}
		}

		if (cfg.mmap_before_spawn) {
			// All requests are in one VMA
			base = mmap(NULL,
				    num_tot_requests * reserved_region_size,
				    PROT_READ | PROT_WRITE,
				    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
			if (base == MAP_FAILED) {
				perror("mmap failed");
				exit(EXIT_FAILURE);
			}

			if (cfg.trigger_fault_before_spawn) {
				// Trigger page faults before spawning threads
				for (int i = 0; i < num_tot_requests; i++) {
					char *region = base + offset[i];
					for (int j = 0;
					     j < cfg.num_pages_per_request; j++)
						region[j * PAGE_SIZE] = 1;
				}
			}
		} else {
			// mmap tests
			if (cfg.is_unfixed_mmap_test) {
				base = NULL;
			} else {
				base = BASE_PTR;
			}
		}

		if (cfg.contention_level == 0) {
			// Low Contention
			// Do nothing.
		} else if (cfg.contention_level == 1) {
			// Random shuffle all
			unsigned int rand = 0xdeadbeef - num_threads;
			for (int i = num_tot_requests - 1; i > 0; i--) {
				rand = simple_get_rand(rand);
				int j = rand % (i + 1);
				long temp = offset[i];
				offset[i] = offset[j];
				offset[j] = temp;
			}
		} else {
			fprintf(stderr, "Invalid Contention Level");
			exit(EXIT_FAILURE);
		}
	}

	// Initialize global variables
	__atomic_clear(&DISPATCH_LIGHT, __ATOMIC_RELEASE);

	// Create threads and trigger page faults in parallel
	for (int i = 0; i < num_threads; i++) {
		thread_data[i].base = base;
		if (offset != NULL) {
			thread_data[i].offset =
				offset + i * cfg.num_requests_per_thread;
		}
		thread_data[i].thread_id = i;
		thread_data[i].tot_threads = num_threads;
		thread_data[i].is_unfixed_mmap_test = cfg.is_unfixed_mmap_test;

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

	// Write throughputs and latencies data to RESULT_FILE
	FILE *file = fopen(RESULT_FILE, "a");
	if (file == NULL) {
		perror("fopen failed");
		exit(EXIT_FAILURE);
	}
	for (int i = 0; i < num_threads; i++) {
		fprintf(file, "\n%ld", cfg.num_requests_per_thread);
		for (int j = 0; j < cfg.num_requests_per_thread; j++) {
			fprintf(file, " %ld", thread_data[i].lat[j]);
		}
		fprintf(file, "\n");
	}
	if (fclose(file) != 0) {
		perror("fclose failed");
		exit(EXIT_FAILURE);
	}
}
