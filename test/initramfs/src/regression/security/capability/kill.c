// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/capability.h"
#include <signal.h>
#include <sys/wait.h>
#include <unistd.h>

FN_TEST(kill_requires_cap_kill_for_other_uid)
{
	int ready_pipe[2];
	int release_pipe[2];
	pid_t pid;
	char ready_byte;
	int status;

	TEST_SUCC(pipe(ready_pipe));
	TEST_SUCC(pipe(release_pipe));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		char byte;

		CHECK(close(ready_pipe[0]));
		CHECK(close(release_pipe[1]));
		CHECK(setresgid(1000, 1000, 1000));
		CHECK(setresuid(1000, 1000, 1000));
		CHECK(write(ready_pipe[1], "r", 1));
		CHECK(close(ready_pipe[1]));
		CHECK(read(release_pipe[0], &byte, sizeof(byte)));
		CHECK(close(release_pipe[0]));
		_exit(EXIT_SUCCESS);
	}

	TEST_SUCC(close(ready_pipe[1]));
	TEST_SUCC(close(release_pipe[0]));
	TEST_SUCC(read(ready_pipe[0], &ready_byte, sizeof(ready_byte)));
	TEST_SUCC(close(ready_pipe[0]));

	TEST_SUCC(kill(pid, 0));

	drop_capability(CAP_KILL);
	TEST_ERRNO(kill(pid, 0), EPERM);

	TEST_SUCC(write(release_pipe[1], "x", 1));
	TEST_SUCC(close(release_pipe[1]));

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()
