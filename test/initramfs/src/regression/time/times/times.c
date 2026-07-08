// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <errno.h>
#include <sys/syscall.h>
#include <sys/times.h>
#include <unistd.h>

FN_TEST(times_valid_tms)
{
	struct tms tms = { 0 };

	TEST_RES(syscall(SYS_times, &tms),
		 _ret >= 0 && tms.tms_utime >= 0 && tms.tms_stime >= 0 &&
			 tms.tms_cutime >= 0 && tms.tms_cstime >= 0);
}
END_TEST()

FN_TEST(times_null_tms)
{
	TEST_RES(syscall(SYS_times, NULL), _ret >= 0);
}
END_TEST()

FN_TEST(times_bad_tms)
{
	TEST_ERRNO(syscall(SYS_times, (void *)0xDEAD0000), EFAULT);
}
END_TEST()
