// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <unistd.h>

#include "../../common/test.h"

#define CHECK_NUMERIC_FILE(path, expected)                                  \
	do {                                                                \
		char buf[64] = { 0 };                                       \
		char *end;                                                  \
		int fd = TEST_SUCC(open((path), O_RDONLY));                 \
		ssize_t bytes_read =                                        \
			TEST_RES(read(fd, buf, sizeof(buf) - 1), _ret > 0); \
                                                                            \
		TEST_SUCC(close(fd));                                       \
		TEST_RES(buf[bytes_read - 1], _ret == '\n');                \
		errno = 0;                                                  \
		unsigned long long value = strtoull(buf, &end, 10);         \
		int parse_errno = errno;                                    \
		TEST_RES(parse_errno, _ret == 0);                           \
		TEST_RES(end > buf, _ret == 1);                             \
		TEST_RES(*end, _ret == '\n');                               \
		TEST_RES(value, _ret == (expected));                        \
	} while (0)

FN_TEST(proc_sys_vm_numeric_files)
{
	CHECK_NUMERIC_FILE("/proc/sys/vm/max_map_count", 1048576);
	CHECK_NUMERIC_FILE("/proc/sys/vm/mmap_min_addr", 65536);
	CHECK_NUMERIC_FILE("/proc/sys/vm/overcommit_memory", 0);
}
END_TEST()
