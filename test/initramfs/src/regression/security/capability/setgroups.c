// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <stdbool.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>

#include "../../common/capability.h"

#define ARRAY_SIZE(array) (sizeof(array) / sizeof((array)[0]))

static int check_setgroups(bool should_drop_capability)
{
	gid_t groups[] = { 0, 65534 };
	int status;

	pid_t pid = CHECK(fork());

	if (pid == 0) {
		if (should_drop_capability) {
			drop_capability(CAP_SETGID);
		}

		int ret = syscall(SYS_setgroups, ARRAY_SIZE(groups), groups);
		if (should_drop_capability) {
			if (ret < 0 && errno == EPERM) {
				_exit(EXIT_SUCCESS);
			}
			_exit(EXIT_FAILURE);
		}

		_exit(ret == 0 ? EXIT_SUCCESS : EXIT_FAILURE);
	}

	CHECK_WITH(waitpid(pid, &status, 0), _ret == pid);

	if (!WIFEXITED(status) || WEXITSTATUS(status) != EXIT_SUCCESS) {
		errno = EINVAL;
		return -1;
	}

	return 0;
}

FN_TEST(setgroups_requires_cap_setgid)
{
	TEST_SUCC(check_setgroups(false));
	TEST_SUCC(check_setgroups(true));
}
END_TEST()
