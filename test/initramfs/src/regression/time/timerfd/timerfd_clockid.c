// SPDX-License-Identifier: MPL-2.0

#include <stdint.h>
#include <sys/timerfd.h>
#include <time.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(timerfd_accepts_supported_clocks)
{
	const clockid_t supported_clocks[] = {
		CLOCK_REALTIME,
		CLOCK_MONOTONIC,
		CLOCK_BOOTTIME,
	};

	for (size_t i = 0;
	     i < sizeof(supported_clocks) / sizeof(supported_clocks[0]); i++) {
		int fd = TEST_RES(timerfd_create(supported_clocks[i], 0),
				  _ret >= 0);
		TEST_SUCC(close(fd));
	}
}
END_TEST()

FN_TEST(timerfd_rejects_fixed_cpu_clocks)
{
	// Linux does not expose CPU-time clocks through timerfd.
	TEST_ERRNO(timerfd_create(CLOCK_PROCESS_CPUTIME_ID, 0), EINVAL);
	TEST_ERRNO(timerfd_create(CLOCK_THREAD_CPUTIME_ID, 0), EINVAL);
}
END_TEST()

FN_TEST(timerfd_rejects_dynamic_clocks_without_panicking)
{
	// The fd clock for fd 0 used to reach an unimplemented kernel path.
	TEST_ERRNO(timerfd_create((clockid_t)-5, 0), EINVAL);

	uint32_t encoded_process_clock = (~(uint32_t)getpid() << 3);
	TEST_ERRNO(timerfd_create((clockid_t)encoded_process_clock, 0), EINVAL);
}
END_TEST()
