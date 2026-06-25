// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <stdlib.h>
#include <unistd.h>

#include "../../common/test.h"

#define DEFAULT_OVERFLOW_ID 65534

static long read_overflow_id(const char *path)
{
	char buf[32];
	char *end;
	int fd = CHECK(open(path, O_RDONLY));
	ssize_t len = CHECK_WITH(read(fd, buf, sizeof(buf) - 1),
				 _ret > 1 && _ret < sizeof(buf));

	CHECK(close(fd));
	buf[len] = '\0';
	CHECK_WITH(buf[len - 1], _ret == '\n');

	long value = strtol(buf, &end, 10);

	CHECK_WITH(*end, _ret == '\n');
	return value;
}

FN_TEST(read_kernel_overflow_ids)
{
	TEST_RES(read_overflow_id("/proc/sys/kernel/overflowuid"),
		 _ret == DEFAULT_OVERFLOW_ID);
	TEST_RES(read_overflow_id("/proc/sys/kernel/overflowgid"),
		 _ret == DEFAULT_OVERFLOW_ID);
}
END_TEST()
