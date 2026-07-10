// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <errno.h>
#include <sys/syscall.h>
#include <sys/times.h>
#include <time.h>
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

FN_TEST(times_matches_sysconf_clk_tck)
{
	struct tms start_tms = { 0 };
	struct tms end_tms = { 0 };
	struct timespec start = { 0 };
	struct timespec now = { 0 };
	long clk_tck = sysconf(_SC_CLK_TCK);
	long elapsed_ns = 0;
	clock_t start_ticks;
	clock_t end_ticks;
	clock_t elapsed_ticks;
	clock_t delta;
	volatile unsigned long sink = 0;

	TEST_RES(clk_tck, clk_tck == 100);
	TEST_SUCC(clock_gettime(CLOCK_MONOTONIC, &start));
	start_ticks = TEST_RES(syscall(SYS_times, &start_tms), _ret >= 0);

	do {
		for (unsigned long i = 0; i < 10000UL; i++) {
			sink += i;
		}
		TEST_SUCC(clock_gettime(CLOCK_MONOTONIC, &now));
		elapsed_ns = (now.tv_sec - start.tv_sec) * 1000000000L +
			     (now.tv_nsec - start.tv_nsec);
	} while (elapsed_ns < 200000000L);

	end_ticks = TEST_RES(syscall(SYS_times, &end_tms), _ret >= 0);
	elapsed_ticks = end_ticks - start_ticks;
	delta = (end_tms.tms_utime + end_tms.tms_stime) -
		(start_tms.tms_utime + start_tms.tms_stime);
	TEST_RES(elapsed_ticks,
		 elapsed_ticks > 0 && elapsed_ticks <= clk_tck + 2);
	TEST_RES(delta, delta >= 0 && delta <= clk_tck + 2);
}
END_TEST()
