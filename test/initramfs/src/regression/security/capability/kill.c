// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <linux/capability.h>
#include <signal.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

static void read_cap_data(struct __user_cap_data_struct cap_data[2])
{
	struct __user_cap_header_struct cap_header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
		.pid = 0,
	};

	CHECK(syscall(SYS_capget, &cap_header, cap_data));
}

static void drop_cap_kill(void)
{
	struct __user_cap_data_struct cap_data[2] = {};

	read_cap_data(cap_data);
	cap_data[0].effective &= ~(1U << CAP_KILL);
	cap_data[0].permitted &= ~(1U << CAP_KILL);
	cap_data[0].inheritable &= ~(1U << CAP_KILL);
	CHECK(syscall(SYS_capset,
		      &(struct __user_cap_header_struct){
			      .version = _LINUX_CAPABILITY_VERSION_3,
			      .pid = 0,
		      },
		      cap_data));
}

FN_TEST(kill_requires_cap_kill_for_other_uid)
{
	int ready_pipe[2];
	int release_pipe[2];
	pid_t pid;
	char ready_byte;
	int status;

	CHECK(pipe(ready_pipe));
	CHECK(pipe(release_pipe));

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

	CHECK(close(ready_pipe[1]));
	CHECK(close(release_pipe[0]));
	CHECK(read(ready_pipe[0], &ready_byte, sizeof(ready_byte)));
	CHECK(close(ready_pipe[0]));

	TEST_SUCC(kill(pid, 0));

	drop_cap_kill();
	TEST_ERRNO(kill(pid, 0), EPERM);

	CHECK(write(release_pipe[1], "x", 1));
	CHECK(close(release_pipe[1]));

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()
