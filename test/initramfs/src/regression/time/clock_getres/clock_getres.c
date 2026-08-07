// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <errno.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

#define FD_TO_CLOCKID(fd) ((~(clockid_t)(fd) << 3) | 3)

FN_TEST(clock_getres_supported_clocks)
{
	struct timespec res = { 0 };

	TEST_RES(syscall(SYS_clock_getres, CLOCK_MONOTONIC, &res),
		 res.tv_sec >= 0 && res.tv_nsec > 0 &&
			 res.tv_nsec < 1000000000L);

	res.tv_sec = 0;
	res.tv_nsec = 0;
	TEST_RES(syscall(SYS_clock_getres, CLOCK_REALTIME, &res),
		 res.tv_sec >= 0 && res.tv_nsec > 0 &&
			 res.tv_nsec < 1000000000L);
}
END_TEST()

FN_TEST(clock_getres_coarse_clocks)
{
	struct timespec res = { 0 };

	TEST_RES(syscall(SYS_clock_getres, CLOCK_MONOTONIC_COARSE, &res),
		 res.tv_sec == 0 && res.tv_nsec == 1000000L);

	res.tv_sec = 0;
	res.tv_nsec = 0;
	TEST_RES(syscall(SYS_clock_getres, CLOCK_REALTIME_COARSE, &res),
		 res.tv_sec == 0 && res.tv_nsec == 1000000L);
}
END_TEST()

FN_TEST(clock_getres_null_res)
{
	TEST_SUCC(syscall(SYS_clock_getres, CLOCK_MONOTONIC, NULL));
}
END_TEST()

FN_TEST(clock_getres_invalid_clock)
{
	struct timespec res = { 0 };

	TEST_ERRNO(syscall(SYS_clock_getres, 0x7fffffff, &res), EINVAL);
}
END_TEST()

FN_TEST(clock_getres_dynamic_fd_clock)
{
	struct timespec res = { 0 };

	TEST_ERRNO(syscall(SYS_clock_getres, FD_TO_CLOCKID(0), &res), EINVAL);
}
END_TEST()
