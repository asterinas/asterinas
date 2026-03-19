// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <string.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(mountstats_contains_proc_mount)
{
	int fd = TEST_SUCC(open("/proc/self/mountstats", O_RDONLY));
	char buf[4096] = { 0 };
	ssize_t bytes_read = TEST_SUCC(read(fd, buf, sizeof(buf) - 1));

	TEST_RES(strstr(buf, "mounted on /proc with fstype proc") != NULL,
		 _ret == 1);
	TEST_RES(bytes_read > 0, _ret == 1);

	TEST_SUCC(close(fd));
}
END_TEST()
