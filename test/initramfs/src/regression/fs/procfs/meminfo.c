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

FN_TEST(slab_field)
{
	int fd = TEST_SUCC(open(MEMINFO_PATH, O_RDONLY));
	char buf[BUF_SIZE];
	ssize_t nread = TEST_SUCC(read(fd, buf, sizeof(buf) - 1));
	TEST_SUCC(close(fd));
	buf[nread] = '\0';

	// Look for the `Slab` line.
	char *line = strstr(buf, "Slab:");
	TEST_RES(line != NULL, _ret == 1);

	// Verify the value is a non-zero number (slab caches should have
	// some allocations).
	char value_str[64];
	int matched = sscanf(line, "Slab:\t%63s kB", value_str);
	TEST_RES(matched == 1, _ret == 1);

	char *end;
	unsigned long value = strtoul(value_str, &end, 10);
	TEST_RES(end != value_str && *end == '\0', _ret == 1);
	TEST_RES(value > 0, _ret == 1);
}
END_TEST()
