// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

FN_TEST(clock_nanosleep_unknown_flag_bit)
{
	struct timespec req = { .tv_sec = 0, .tv_nsec = 0 };

	// The flag (`0xdeadbeef`) is neither 0 nor `TIMER_ABSTIME`.
	// Linux will ignore unknown flag bits.
	TEST_SUCC(syscall(SYS_clock_nanosleep, CLOCK_MONOTONIC, 0xdeadbeef,
			  &req, NULL));
}
END_TEST()
