// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <stdint.h>
#include <fcntl.h>
#include <limits.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/syscall.h>
#include <unistd.h>

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

FN_TEST(dup_invalid_old_preserves_new)
{
	char buf[1];
	int new_fd = CHECK(dup(fd));

	TEST_ERRNO(dup2(-1, new_fd), EBADF);
	TEST_RES(read(new_fd, buf, sizeof(buf)), _ret == 1);

	TEST_ERRNO(syscall(SYS_dup3, -1, new_fd, 0), EBADF);
	TEST_RES(read(new_fd, buf, sizeof(buf)), _ret == 1);

	TEST_SUCC(close(new_fd));
}
END_TEST()

FN_TEST(dupfd_exceeds_rlimit)
{
	struct rlimit old_rlim, rlim;

	TEST_SUCC(getrlimit(RLIMIT_NOFILE, &old_rlim));
	rlim = old_rlim;
	rlim.rlim_cur = 64;
	TEST_SUCC(setrlimit(RLIMIT_NOFILE, &rlim));

	// `F_DUPFD` and `F_DUPFD_CLOEXEC` fail with `EINVAL` if the minimum FD is
	// not below `RLIMIT_NOFILE`, instead of growing the FD table to that size.
	TEST_ERRNO(fcntl(fd, F_DUPFD, 64), EINVAL);
	TEST_ERRNO(fcntl(fd, F_DUPFD_CLOEXEC, 64), EINVAL);
	TEST_ERRNO(fcntl(fd, F_DUPFD, INT_MAX), EINVAL);

	TEST_RES(fcntl(fd, F_DUPFD, 63), _ret == 63);
	TEST_ERRNO(fcntl(fd, F_DUPFD, 63), EMFILE);
	TEST_ERRNO(fcntl(fd, F_DUPFD_CLOEXEC, 63), EMFILE);
	TEST_SUCC(close(63));
	TEST_RES(fcntl(fd, F_DUPFD_CLOEXEC, 63), _ret == 63);
	TEST_RES(fcntl(63, F_GETFD), _ret == FD_CLOEXEC);
	TEST_SUCC(close(63));

	// Source-FD validation takes precedence over either invalid minimum.
	TEST_ERRNO(fcntl(63, F_DUPFD, 64), EBADF);
	TEST_ERRNO(fcntl(63, F_DUPFD_CLOEXEC, 64), EBADF);
	TEST_ERRNO(fcntl(63, F_DUPFD, -1), EBADF);
	TEST_ERRNO(fcntl(63, F_DUPFD_CLOEXEC, -1), EBADF);
	TEST_ERRNO(fcntl(fd, F_DUPFD, -1), EINVAL);
	TEST_ERRNO(fcntl(fd, F_DUPFD_CLOEXEC, -1), EINVAL);

	TEST_SUCC(setrlimit(RLIMIT_NOFILE, &old_rlim));
}
END_TEST()

FN_TEST(dup_zero_rlimit)
{
	struct rlimit old_rlim, rlim;

	int closed_fd = TEST_SUCC(dup(fd));
	TEST_SUCC(close(closed_fd));

	TEST_SUCC(getrlimit(RLIMIT_NOFILE, &old_rlim));
	rlim = old_rlim;
	rlim.rlim_cur = 0;
	TEST_SUCC(setrlimit(RLIMIT_NOFILE, &rlim));

	// Existing FDs remain valid, but no new FD can be allocated.
	TEST_ERRNO(dup(fd), EMFILE);
	TEST_ERRNO(fcntl(fd, F_DUPFD, 0), EINVAL);
	TEST_ERRNO(fcntl(fd, F_DUPFD_CLOEXEC, 0), EINVAL);
	TEST_ERRNO(dup(closed_fd), EBADF);
	TEST_ERRNO(fcntl(closed_fd, F_DUPFD, 0), EBADF);
	TEST_ERRNO(fcntl(closed_fd, F_DUPFD_CLOEXEC, 0), EBADF);

	TEST_SUCC(setrlimit(RLIMIT_NOFILE, &old_rlim));
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
