// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

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
