// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../network/test.h"
#include <signal.h>
#include <string.h>
#include <sys/poll.h>
#include <sched.h>

FN_SETUP()
{
}
END_SETUP()

FN_TEST(yield)
{
	TEST_RES(sched_yield(), _ret == 0);
}
END_TEST()