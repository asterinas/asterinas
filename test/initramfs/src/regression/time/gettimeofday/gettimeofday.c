// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <errno.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <unistd.h>

FN_TEST(gettimeofday_invalid_tz_efault)
{
	TEST_ERRNO(syscall(SYS_gettimeofday, NULL, (void *)0xDEAD0000), EFAULT);
}
END_TEST()

FN_TEST(gettimeofday_invalid_tv_efault)
{
	TEST_ERRNO(syscall(SYS_gettimeofday, (void *)0xDEAD0000, NULL), EFAULT);
}
END_TEST()

FN_TEST(gettimeofday_both_null_success)
{
	TEST_SUCC(syscall(SYS_gettimeofday, NULL, NULL));
}
END_TEST()

FN_TEST(gettimeofday_valid_tv)
{
	struct timeval tv = { 0 };
	TEST_SUCC(syscall(SYS_gettimeofday, &tv, NULL));
}
END_TEST()

FN_TEST(gettimeofday_valid_tz)
{
	struct timezone tz = { 0 };
	TEST_SUCC(syscall(SYS_gettimeofday, NULL, &tz));
}
END_TEST()
