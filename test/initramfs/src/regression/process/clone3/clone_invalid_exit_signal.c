// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <linux/sched.h>
#include <sys/wait.h>
#include <sys/syscall.h>
#include <stdlib.h>
#include <unistd.h>

#include "../../common/test.h"

static pid_t sys_clone3(struct clone_args *args)
{
	return syscall(SYS_clone3, args, sizeof(struct clone_args));
}

FN_TEST(clone_normalizes_invalid_exit_signal)
{
	int status;
	long pid;

	pid = TEST_SUCC(syscall(SYS_clone, 255UL, 0UL, 0UL, 0UL, 0UL));

	if (pid == 0) {
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_TEST(clone3_rejects_invalid_exit_signal)
{
	struct clone_args args = {};

	args.exit_signal = 0xff;
	TEST_ERRNO(sys_clone3(&args), EINVAL);

	args.exit_signal = 0x101;
	TEST_ERRNO(sys_clone3(&args), EINVAL);
}
END_TEST()