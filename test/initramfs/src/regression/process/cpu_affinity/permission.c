// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <sched.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_UID 5000

FN_TEST(sched_setaffinity_permissions)
{
	cpu_set_t mask;
	int ready[2];
	int proceed[2];
	char byte = 0;

	CHECK(sched_getaffinity(0, sizeof(mask), &mask));
	CHECK(pipe(ready));
	CHECK(pipe(proceed));
	pid_t pid = CHECK(fork());
	if (pid == 0) {
		CHECK(close(ready[0]));
		CHECK(close(proceed[1]));
		CHECK(setresgid(TEST_UID, TEST_UID, TEST_UID));
		CHECK(setresuid(TEST_UID, TEST_UID, TEST_UID));
		CHECK(sched_setaffinity(0, sizeof(mask), &mask));
		CHECK(sched_setaffinity(getpid(), sizeof(mask), &mask));
		CHECK_WITH(sched_setaffinity(getppid(), sizeof(mask), &mask),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(write(ready[1], &byte, 1), _ret == 1);
		CHECK_WITH(read(proceed[0], &byte, 1), _ret == 1);
		_exit(EXIT_SUCCESS);
	}
	CHECK(close(ready[1]));
	CHECK(close(proceed[0]));
	CHECK_WITH(read(ready[0], &byte, 1), _ret == 1);
	TEST_SUCC(sched_setaffinity(pid, sizeof(mask), &mask));
	CHECK_WITH(write(proceed[1], &byte, 1), _ret == 1);
	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
	CHECK(close(ready[0]));
	CHECK(close(proceed[1]));
}
END_TEST()
