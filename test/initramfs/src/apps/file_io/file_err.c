// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../test.h"
#include <stdint.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>

static int fd;

FN_SETUP(open)
{
	fd = CHECK(open("/etc/passwd", O_RDONLY));
}
END_SETUP()

FN_TEST(dup_out_of_range)
{
	// `-1` is out of the allowed range for file descriptors.
	TEST_ERRNO(dup2(fd, -1), EBADF);
}
END_TEST()

FN_TEST(flock_overflow)
{
	struct flock fl;

	fl.l_type = F_RDLCK;
	fl.l_whence = SEEK_SET;

	// `l_start + l_len` underflows.
	fl.l_start = -1;
	fl.l_len = INT64_MIN;
	TEST_ERRNO(fcntl(fd, F_SETLK, &fl), EINVAL);

	// `l_start + l_len` overflows.
	fl.l_start = 2;
	fl.l_len = INT64_MAX;
	TEST_ERRNO(fcntl(fd, F_SETLK, &fl), EOVERFLOW);
}
END_TEST()

FN_TEST(ftruncate_large)
{
	int memfd;

	memfd = TEST_SUCC(memfd_create("test_memfd", 0));

	// `ftruncate` can handle large expansions and shrinking.
	TEST_SUCC(ftruncate(memfd, ((off_t)1) << 50));
	TEST_SUCC(ftruncate(memfd, 0));

	TEST_SUCC(close(memfd));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(fd));
}
END_SETUP()
