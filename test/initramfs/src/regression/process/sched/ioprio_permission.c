// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define IOPRIO_WHO_PROCESS 1
#define IOPRIO_CLASS_SHIFT 13
#define IOPRIO_CLASS_RT 1
#define IOPRIO_CLASS_BE 2
#define TEST_UID 5000
#define BE_PRIORITY ((IOPRIO_CLASS_BE << IOPRIO_CLASS_SHIFT) | 4)
#define OTHER_BE_PRIORITY ((IOPRIO_CLASS_BE << IOPRIO_CLASS_SHIFT) | 3)
#define RT_PRIORITY ((IOPRIO_CLASS_RT << IOPRIO_CLASS_SHIFT) | 4)

FN_TEST(ioprio_set_permissions)
{
	int ready[2];
	int proceed[2];
	char byte = 0;
	long original = CHECK(syscall(SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0));

	TEST_SUCC(syscall(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, RT_PRIORITY));
	CHECK(syscall(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, original));
	CHECK(pipe(ready));
	CHECK(pipe(proceed));
	pid_t pid = CHECK(fork());
	if (pid == 0) {
		CHECK(close(ready[0]));
		CHECK(close(proceed[1]));
		CHECK(setresgid(TEST_UID, TEST_UID, TEST_UID));
		CHECK(setresuid(TEST_UID, TEST_UID, TEST_UID));
		CHECK(syscall(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0,
			      BE_PRIORITY));
		CHECK_WITH(syscall(SYS_ioprio_set, IOPRIO_WHO_PROCESS,
				   getppid(), BE_PRIORITY),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(syscall(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0,
				   RT_PRIORITY),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(write(ready[1], &byte, 1), _ret == 1);
		CHECK_WITH(read(proceed[0], &byte, 1), _ret == 1);
		_exit(EXIT_SUCCESS);
	}
	CHECK(close(ready[1]));
	CHECK(close(proceed[0]));
	CHECK_WITH(read(ready[0], &byte, 1), _ret == 1);
	TEST_SUCC(syscall(SYS_ioprio_set, IOPRIO_WHO_PROCESS, pid,
			  OTHER_BE_PRIORITY));
	TEST_RES(syscall(SYS_ioprio_get, IOPRIO_WHO_PROCESS, pid),
		 _ret == OTHER_BE_PRIORITY);
	TEST_RES(syscall(SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0),
		 _ret == original);
	CHECK_WITH(write(proceed[1], &byte, 1), _ret == 1);
	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
	CHECK(close(ready[0]));
	CHECK(close(proceed[1]));
}
END_TEST()
