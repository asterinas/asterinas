// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/capability.h"

#define TEST_UID 5000

FN_TEST(setpriority_permissions)
{
	int ready[2];
	int proceed[2];
	char byte = 0;
	int original = TEST_SUCC(getpriority(PRIO_PROCESS, 0));

	TEST_SUCC(pipe(ready));
	TEST_SUCC(pipe(proceed));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(close(ready[0]));
		CHECK(close(proceed[1]));

		CHECK(setresgid(TEST_UID, TEST_UID, TEST_UID));
		CHECK(setresuid(TEST_UID, TEST_UID, TEST_UID));

		CHECK(setpriority(PRIO_PROCESS, 0, 10));
		CHECK_WITH(setpriority(PRIO_PROCESS, getppid(), 19),
			   _ret == -1 && errno == EPERM);

		CHECK_WITH(write(ready[1], &byte, 1), _ret == 1);
		CHECK_WITH(read(proceed[0], &byte, 1), _ret == 1);

		CHECK(close(ready[1]));
		CHECK(close(proceed[0]));
		_exit(EXIT_SUCCESS);
	}

	TEST_SUCC(close(ready[1]));
	TEST_SUCC(close(proceed[0]));

	TEST_RES(read(ready[0], &byte, 1), _ret == 1);

	TEST_RES(getpriority(PRIO_PROCESS, 0), _ret == original);
	TEST_RES(getpriority(PRIO_PROCESS, pid), _ret == 10);

	TEST_SUCC(setpriority(PRIO_PROCESS, pid, 19));
	TEST_RES(getpriority(PRIO_PROCESS, pid), _ret == 19);

	TEST_RES(write(proceed[1], &byte, 1), _ret == 1);

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);

	TEST_SUCC(close(ready[0]));
	TEST_SUCC(close(proceed[1]));
}
END_TEST()

FN_TEST(setpriority_capability)
{
	struct rlimit64 limit = {
		.rlim_cur = 8, /* soft: prio=8, nice=12 */
		.rlim_max = 12, /* hard: prio=12, nice=8 */
	};

	TEST_SUCC(setrlimit64(RLIMIT_NICE, &limit));

	/* Decreasing the nice value is always okay with CAP_SYS_NICE. */
	TEST_SUCC(setpriority(PRIO_PROCESS, 0, 14));
	TEST_SUCC(setpriority(PRIO_PROCESS, 0, 10));
	TEST_SUCC(setpriority(PRIO_PROCESS, 0, 6));

	TEST_RES(getpriority(PRIO_PROCESS, 0), _ret == 6);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		drop_capability(CAP_SYS_NICE);

		/* Increasing the nice value is always okay. */
		CHECK(setpriority(PRIO_PROCESS, 0, 6));
		CHECK(setpriority(PRIO_PROCESS, 0, 10));
		CHECK(setpriority(PRIO_PROCESS, 0, 14));

		/* Decreasing the nice value may not be okay without CAP_SYS_NICE. */
		CHECK(setpriority(PRIO_PROCESS, 0, 13));
		CHECK(setpriority(PRIO_PROCESS, 0, 12));
		CHECK_WITH(setpriority(PRIO_PROCESS, 0, 11),
			   _ret == -1 && errno == EACCES);

		CHECK_WITH(getpriority(PRIO_PROCESS, 0), _ret == 12);

		exit(EXIT_SUCCESS);
	}

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()
