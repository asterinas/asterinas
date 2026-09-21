// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <signal.h>
#include <string.h>
#include <unistd.h>
#include <sys/syscall.h>

#include "../../common/test.h"

static long rt_sigpending(void *set, size_t sigsetsize)
{
	return syscall(SYS_rt_sigpending, set, sigsetsize);
}

sigset_t blocked;

FN_SETUP(block_sigusr1)
{
	sigemptyset(&blocked);
	sigaddset(&blocked, SIGUSR1);
	CHECK(sigprocmask(SIG_BLOCK, &blocked, NULL));
	CHECK(raise(SIGUSR1));
}
END_SETUP()

FN_TEST(sigsetsize_larger_than_eight_rejected)
{
	unsigned char buf[16];

	TEST_ERRNO(rt_sigpending(buf, 9), EINVAL);
	TEST_ERRNO(rt_sigpending(buf, 16), EINVAL);
}
END_TEST()

FN_TEST(sigsetsize_equal_to_eight)
{
	unsigned long val = 0;

	TEST_SUCC(rt_sigpending(&val, 8));
	TEST_RES(rt_sigpending(&val, 8), val & (1UL << (SIGUSR1 - 1)));
}
END_TEST()

FN_TEST(sigsetsize_smaller_than_eight)
{
	unsigned char buf[8];

	// sigsetsize = 4: should write the lower 4 bytes containing SIGUSR1
	memset(buf, 0, sizeof(buf));
	TEST_RES(rt_sigpending(buf, 4),
		 buf[((SIGUSR1 - 1) / 8)] & (1U << ((SIGUSR1 - 1) % 8)));

	// sigsetsize = 0: no-op, should succeed
	TEST_SUCC(rt_sigpending(buf, 0));
}
END_TEST()

FN_TEST(null_pointer_rejected)
{
	TEST_ERRNO(rt_sigpending(NULL, 8), EFAULT);
}
END_TEST()

FN_SETUP(cleanup)
{
	signal(SIGUSR1, SIG_IGN);
	CHECK(sigprocmask(SIG_UNBLOCK, &blocked, NULL));
}
END_SETUP()
