// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <sys/fcntl.h>
#include <sys/mman.h>
#include <sys/mount.h>
#include <unistd.h>

#include "../../common/test.h"

#define PAGE_SIZE 4096

static unsigned long get_vm_rss_kb(void)
{
	FILE *status =
		CHECK_WITH(fopen("/proc/self/status", "r"), _ret != NULL);
	char line[256];
	unsigned long vm_rss_kb = 0;
	bool found_vm_rss = false;
	while (fgets(line, sizeof(line), status)) {
		if (sscanf(line, "VmRSS: %lu", &vm_rss_kb) == 1) {
			found_vm_rss = true;
			break;
		}
	}
	CHECK(fclose(status));
	CHECK_WITH(found_vm_rss, _ret);
	return vm_rss_kb;
}

FN_TEST(mmap_populate_short_file)
{
	const char *filename = "mmap_populate_short_file";
	int fd = TEST_SUCC(open(filename, O_CREAT | O_RDWR | O_TRUNC, 0600));
	TEST_RES(write(fd, "a", 1), _ret == 1);

	char *private_addr =
		TEST_SUCC(mmap(NULL, 2 * PAGE_SIZE, PROT_READ | PROT_WRITE,
			       MAP_PRIVATE | MAP_POPULATE, fd, 0));
	private_addr[0] = 'b';

	char file_byte;
	TEST_RES(pread(fd, &file_byte, 1, 0), _ret == 1);
	TEST_RES(file_byte, _ret == 'a');
	TEST_SUCC(munmap(private_addr, 2 * PAGE_SIZE));

	char *shared_addr = TEST_SUCC(mmap(NULL, 2 * PAGE_SIZE, PROT_READ,
					   MAP_SHARED | MAP_POPULATE, fd, 0));
	TEST_RES(shared_addr[0], _ret == 'a');
	TEST_SUCC(munmap(shared_addr, 2 * PAGE_SIZE));

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(filename));
}
END_TEST()

FN_TEST(mmap_populate_prot_none)
{
	bool mounted_proc = false;
	if (access("/proc/self/statm", F_OK) < 0) {
		TEST_SUCC(mount("proc", "/proc", "proc", 0, NULL));
		mounted_proc = true;
	}

	unsigned long rss_before = get_vm_rss_kb();

	const size_t map_size = 32 * 1024 * 1024;
	char *addr = TEST_SUCC(mmap(NULL, map_size, PROT_NONE,
				    MAP_PRIVATE | MAP_ANONYMOUS | MAP_POPULATE,
				    -1, 0));

	unsigned long rss_after = get_vm_rss_kb();
	TEST_RES(rss_after, _ret <= rss_before + 4 * 1024);

	TEST_SUCC(munmap(addr, map_size));
	if (mounted_proc)
		TEST_SUCC(umount("/proc"));
}
END_TEST()
