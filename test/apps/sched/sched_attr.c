// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../test.h"
#include <signal.h>
#include <string.h>
#include <sys/poll.h>
#include <sched.h>

FN_SETUP()
{
}
END_SETUP()

FN_TEST(sched_param)
{
	TEST_ERRNO(sched_getscheduler(-100), EINVAL);
	TEST_ERRNO(sched_getscheduler(1234567890), ESRCH);
	TEST_RES(sched_getscheduler(0), _ret == SCHED_OTHER);

	struct sched_param param;

	TEST_ERRNO(sched_getparam(0, NULL), EINVAL);
	TEST_RES(sched_getparam(0, &param),
		 _ret == 0 && param.sched_priority == 0);

	param.sched_priority = 50;
	TEST_ERRNO(sched_setscheduler(0, SCHED_FIFO, NULL), EINVAL);
	TEST_ERRNO(sched_setscheduler(0, 1234567890, &param), EINVAL);
	TEST_ERRNO(sched_setscheduler(-100, SCHED_FIFO, &param), EINVAL);
	TEST_ERRNO(sched_setscheduler(1234567890, SCHED_FIFO, &param), ESRCH);
	TEST_RES(sched_setscheduler(0, SCHED_FIFO, &param), _ret == 0);
	sleep(1);

	TEST_RES(sched_getscheduler(0), _ret == SCHED_FIFO);
	TEST_RES(sched_getparam(0, &param),
		 _ret == 0 && param.sched_priority == 50);

	param.sched_priority = 1234567890;
	TEST_ERRNO(sched_setparam(0, NULL), EINVAL);
	TEST_ERRNO(sched_setparam(-100, &param), EINVAL);
	TEST_ERRNO(sched_setparam(1234567890, &param), ESRCH);
	TEST_ERRNO(sched_setparam(0, &param), EINVAL);
	param.sched_priority = 51;
	TEST_RES(sched_setparam(0, &param), _ret == 0);
	sleep(1);

	TEST_RES(sched_getparam(0, &param),
		 _ret == 0 && param.sched_priority == 51);
}
END_TEST()

FN_TEST(sched_prio_limit)
{
	TEST_ERRNO(sched_get_priority_max(-100), EINVAL);
	TEST_ERRNO(sched_get_priority_max(1234567890), EINVAL);
	TEST_ERRNO(sched_get_priority_min(-100), EINVAL);
	TEST_ERRNO(sched_get_priority_min(1234567890), EINVAL);

	TEST_RES(sched_get_priority_max(SCHED_OTHER), _ret == 0);
	TEST_RES(sched_get_priority_min(SCHED_OTHER), _ret == 0);

	TEST_RES(sched_get_priority_max(SCHED_FIFO), _ret == 99);
	TEST_RES(sched_get_priority_min(SCHED_FIFO), _ret == 1);

	TEST_RES(sched_get_priority_max(SCHED_RR), _ret == 99);
	TEST_RES(sched_get_priority_min(SCHED_RR), _ret == 1);

#ifdef __USE_GNU
	TEST_RES(sched_get_priority_max(SCHED_IDLE), _ret == 0);
	TEST_RES(sched_get_priority_min(SCHED_IDLE), _ret == 0);
#endif
}
END_TEST()