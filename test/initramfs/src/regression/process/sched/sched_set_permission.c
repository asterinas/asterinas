// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <linux/sched/types.h>
#include <sched.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_UID 5000

FN_TEST(sched_set_permissions)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		struct sched_param param = { .sched_priority = 0 };
		struct sched_attr attr = {
			.size = sizeof(attr),
			.sched_policy = SCHED_OTHER,
		};

		CHECK(sched_setparam(getppid(), &param));
		CHECK(sched_setscheduler(getppid(), SCHED_OTHER, &param));
		CHECK(syscall(SYS_sched_setattr, getppid(), &attr, 0));

		CHECK(setgid(TEST_UID));
		CHECK(setuid(TEST_UID));

		CHECK(sched_setparam(0, &param));
		CHECK(sched_setscheduler(0, SCHED_OTHER, &param));
		CHECK(syscall(SYS_sched_setattr, 0, &attr, 0));

		CHECK_WITH(sched_setparam(getppid(), &param),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(sched_setscheduler(getppid(), SCHED_OTHER, &param),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(syscall(SYS_sched_setattr, getppid(), &attr, 0),
			   _ret == -1 && errno == EPERM);

		_exit(EXIT_SUCCESS);
	}

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);

	TEST_RES(sched_getscheduler(0), _ret == SCHED_OTHER);
}
END_TEST()
