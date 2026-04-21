// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>

#include "../../common/test.h"

FN_TEST(invalid_fd)
{
	TEST_ERRNO(open("/proc/self/fd/2147483647", O_RDONLY), ENOENT);
	TEST_ERRNO(open("/proc/self/fd/2147483648", O_RDONLY), ENOENT);
	TEST_ERRNO(open("/proc/self/fd/2147483649", O_RDONLY), ENOENT);

	TEST_ERRNO(open("/proc/self/fd/4294967295", O_RDONLY), ENOENT);
	TEST_ERRNO(open("/proc/self/fd/4294967296", O_RDONLY), ENOENT);
	TEST_ERRNO(open("/proc/self/fd/4294967297", O_RDONLY), ENOENT);

	TEST_ERRNO(open("/proc/self/fd/-1", O_RDONLY), ENOENT);
	TEST_ERRNO(open("/proc/self/fd/-2147483648", O_RDONLY), ENOENT);
	TEST_ERRNO(open("/proc/self/fd/-4294967296", O_RDONLY), ENOENT);
}
END_TEST()
