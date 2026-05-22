// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <stdbool.h>
#include <stdlib.h>
#include <unistd.h>
#include <linux/capability.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>

#include "../../common/test.h"

#define ARRAY_SIZE(array) (sizeof(array) / sizeof((array)[0]))

static void drop_cap_setgid(void)
{
	struct __user_cap_header_struct header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
	};
	struct __user_cap_data_struct data[2] = { 0 };

	CHECK(syscall(SYS_capget, &header, data));

	data[0].effective &= ~(1 << CAP_SETGID);
	data[0].permitted &= ~(1 << CAP_SETGID);
	data[0].inheritable &= ~(1 << CAP_SETGID);

	CHECK(syscall(SYS_capset, &header, data));
}

static int check_setgroups(bool drop_capability)
{
	gid_t groups[] = { 0, 65534 };
	int status;

	pid_t pid = CHECK(fork());

	if (pid == 0) {
		if (drop_capability) {
			drop_cap_setgid();
		}

		int ret = syscall(SYS_setgroups, ARRAY_SIZE(groups), groups);
		if (drop_capability) {
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
