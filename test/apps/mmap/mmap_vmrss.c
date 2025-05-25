// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
#include <errno.h>

#define PAGE_SIZE 4096
#define NUM_PAGES 16
#define TOTAL_SIZE (PAGE_SIZE * NUM_PAGES)

long get_vm_rss_kb()
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
	while (fgets(line, sizeof(line), f)) {
		if (strncmp(line, "VmRSS:", 6) == 0) {
			sscanf(line + 6, "%ld", &rss_kb);
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

int main()
{
	void *mem = mmap(NULL, TOTAL_SIZE, PROT_READ | PROT_WRITE,
			 MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	if (mem == MAP_FAILED) {
		perror("mmap");
		exit(1);
	}

	long rss_before = get_vm_rss_kb();
	// The first call to `get_vm_rss_kb()` may trigger lazy mapping of
	// additional pages, such as shared libraries or files. These pages
	// are not counted in RSS until they are actually accessed. By
	// calling it again, we can ensure that the second call returns the
	// accurate RSS.
	rss_before = get_vm_rss_kb();

	// Trigger page faults
	for (int i = 0; i < NUM_PAGES; ++i) {
		volatile char *p = (char *)mem + i * PAGE_SIZE;
		*p = 42;
	}

	long rss_after = get_vm_rss_kb();

	long diff_kb = rss_after - rss_before;

	if (diff_kb != NUM_PAGES * (PAGE_SIZE / 1024)) {
		fprintf(stderr, "VmRSS increased by %ld KB, expected %d KB\n",
			diff_kb, NUM_PAGES * (PAGE_SIZE / 1024));
		perror("VmRSS mismatch");
		exit(1);
	}

	printf("VmRSS increased as expected by %ld KB.\n", diff_kb);

	munmap(mem, TOTAL_SIZE);
	return 0;
}
