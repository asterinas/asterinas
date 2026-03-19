// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
#include <fcntl.h>

#define PAGE_SIZE 4096
#define NUM_PAGES 1024
#define TOTAL_SIZE (PAGE_SIZE * NUM_PAGES)

// The RSS count in Linux may differ slightly from the expected value,
// because its per-CPU counters are updated lazily.
#ifdef __asterinas__
long get_rss_deviation_kb()
{
	return 0;
}
#else
long get_rss_deviation_kb()
{
	static long cached_deviation_kb = -1;

	if (cached_deviation_kb >= 0)
		return cached_deviation_kb;

	// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/lib/percpu_counter.c#L258-L264>.
	long nrcpus = CHECK(sysconf(_SC_NPROCESSORS_ONLN));
	long batch = nrcpus * 2;
	if (batch < 32)
		batch = 32;
	// Anon pages and file pages are counted in separate per-CPU counters.
	cached_deviation_kb = batch * 2 * (PAGE_SIZE / 1024);
	return cached_deviation_kb;
}
#endif

typedef enum rss_type {
	anon,
	file,
	total,
} rss_type;

long get_vm_rss_kb(rss_type type)
{
	pid_t pid = getpid();
	char path[64];
	snprintf(path, sizeof(path), "/proc/%d/status", pid);

	FILE *f = fopen(path, "r");
	if (!f) {
		perror("fopen /proc/[pid]/status");
		exit(1);
	}

	char line[256];
	long rss_kb = -1;
	const char *target_field = NULL;
	switch (type) {
	case anon:
		target_field = "RssAnon:";
		break;
	case file:
		target_field = "RssFile:";
		break;
	case total:
		target_field = "VmRSS:";
		break;
	default:
		fprintf(stderr, "Unknown rss_type\n");
		exit(1);
	}

	while (fgets(line, sizeof(line), f)) {
		if (strncmp(line, target_field, strlen(target_field)) == 0) {
			sscanf(line + strlen(target_field), "%ld", &rss_kb);
			break;
		}
	}

	fclose(f);

	if (rss_kb < 0) {
		fprintf(stderr, "Failed to parse VmRSS\n");
		exit(1);
	}

	return rss_kb;
}

// Tests whether the returned RSS value is within the expected range.
#define TEST_RSS(expr, expected) \
	TEST_RES(expr, (labs(_ret - (expected)) <= get_rss_deviation_kb()))

FN_TEST(rss_anon)
{
	void *mem = TEST_SUCC(mmap(NULL, TOTAL_SIZE, PROT_READ | PROT_WRITE,
				   MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));

	// The first call to `TEST_SUCC` and `get_vm_rss_kb()` may trigger
	// lazy mapping of additional pages, such as shared libraries or files.
	// These pages are not counted in RSS until they are actually accessed.
	TEST_SUCC(get_vm_rss_kb(anon));

	long rss_anon_before = TEST_SUCC(get_vm_rss_kb(anon));
	long rss_file_before = TEST_SUCC(get_vm_rss_kb(file));
	long rss_before = TEST_SUCC(get_vm_rss_kb(total));

	// Trigger page faults
	for (int i = 0; i < NUM_PAGES; ++i) {
		volatile char *p = (char *)mem + i * PAGE_SIZE;
		*p = 42;
	}

	TEST_RSS(get_vm_rss_kb(anon),
		 rss_anon_before + NUM_PAGES * (PAGE_SIZE / 1024));
	TEST_RSS(get_vm_rss_kb(file), rss_file_before);
	TEST_RSS(get_vm_rss_kb(total),
		 rss_before + NUM_PAGES * (PAGE_SIZE / 1024));

	TEST_SUCC(munmap(mem, TOTAL_SIZE));

	TEST_RSS(get_vm_rss_kb(anon), rss_anon_before);
	TEST_RSS(get_vm_rss_kb(file), rss_file_before);
	TEST_RSS(get_vm_rss_kb(total), rss_before);
}
END_TEST()

FN_TEST(rss_file)
{
	const char *filename = "rss_test_file";
	int fd = TEST_SUCC(open(filename, O_CREAT | O_RDWR, 0600));

	TEST_SUCC(ftruncate(fd, TOTAL_SIZE));

	// The first call to `TEST_SUCC` and `get_vm_rss_kb()` may trigger
	// lazy mapping of additional pages, such as shared libraries or files.
	// These pages are not counted in RSS until they are actually accessed.
	TEST_SUCC(get_vm_rss_kb(anon));

	long rss_anon_before = TEST_SUCC(get_vm_rss_kb(anon));
	long rss_file_before = TEST_SUCC(get_vm_rss_kb(file));
	long rss_before = TEST_SUCC(get_vm_rss_kb(total));

	void *mem = TEST_SUCC(mmap(NULL, TOTAL_SIZE, PROT_READ | PROT_WRITE,
				   MAP_PRIVATE, fd, 0));

	// Trigger page faults
	for (int i = 0; i < NUM_PAGES; ++i) {
		volatile char *p = (char *)mem + i * PAGE_SIZE;
		(void)*p;
	}

	TEST_RSS(get_vm_rss_kb(anon), rss_anon_before);
	TEST_RSS(get_vm_rss_kb(file),
		 rss_file_before + NUM_PAGES * (PAGE_SIZE / 1024));
	TEST_RSS(get_vm_rss_kb(total),
		 rss_before + NUM_PAGES * (PAGE_SIZE / 1024));

	// Trigger COW
	for (int i = 0; i < NUM_PAGES; ++i) {
		volatile char *p = (char *)mem + i * PAGE_SIZE;
		*p = 42;
	}

	// Linux re-classifies a page from `File` to `Anon` in RSS,
	// when a COW on a file-backed mapping happens.
	// Currently Asterinas does not support this re-classification.
#ifdef __asterinas__
	TEST_RSS(get_vm_rss_kb(anon), rss_anon_before);
	TEST_RSS(get_vm_rss_kb(file),
		 rss_file_before + NUM_PAGES * (PAGE_SIZE / 1024));
#else
	TEST_RSS(get_vm_rss_kb(anon),
		 rss_anon_before + NUM_PAGES * (PAGE_SIZE / 1024));
	TEST_RSS(get_vm_rss_kb(file), rss_file_before);
#endif

	TEST_RSS(get_vm_rss_kb(total),
		 rss_before + NUM_PAGES * (PAGE_SIZE / 1024));

	TEST_SUCC(munmap(mem, TOTAL_SIZE));

	TEST_RSS(get_vm_rss_kb(anon), rss_anon_before);
	TEST_RSS(get_vm_rss_kb(file), rss_file_before);
	TEST_RSS(get_vm_rss_kb(total), rss_before);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(filename));
}
END_TEST()
