// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>

#include "../../common/test.h"

#define MEMINFO_PATH "/proc/meminfo"
#define BUF_SIZE 4096

FN_TEST(kernel_heap_field)
{
	int fd = TEST_SUCC(open(MEMINFO_PATH, O_RDONLY));
	char buf[BUF_SIZE];
	ssize_t nread = TEST_SUCC(read(fd, buf, sizeof(buf) - 1));
	TEST_SUCC(close(fd));
	buf[nread] = '\0';

	// Look for the `KernelHeap` line.
	char *line = strstr(buf, "KernelHeap:");
	TEST_RES(line != NULL, _ret == 1);

	// Verify the value is non-zero (heap should have some allocations).
	char value_str[64];
	int matched = sscanf(line, "KernelHeap:\t%63s kB", value_str);
	TEST_RES(matched == 1, _ret == 1);

	unsigned long value = strtoul(value_str, NULL, 10);
	TEST_RES(value > 0, _ret == 1);
}
END_TEST()
