// SPDX-License-Identifier: MPL-2.0

#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define PATH_BUF_SIZE 4096

static int readlink_into(const char *path, char *buf, size_t size)
{
	ssize_t len = readlink(path, buf, size - 1);

	if (len <= 0 || len >= (ssize_t)size) {
		return -1;
	}
	buf[len] = '\0';
	return 0;
}

FN_TEST(proc_self_cwd)
{
	char want[PATH_BUF_SIZE];
	char got[PATH_BUF_SIZE];

	TEST_RES(getcwd(want, sizeof(want)) != NULL, _ret != 0);
	TEST_RES(readlink_into("/proc/self/cwd", got, sizeof(got)), _ret == 0);
	TEST_RES(strcmp(got, want), _ret == 0);
}
END_TEST()

FN_TEST(proc_pid_cwd)
{
	char want[PATH_BUF_SIZE];
	char path[PATH_BUF_SIZE];
	char got[PATH_BUF_SIZE];
	pid_t child;

	TEST_RES(getcwd(want, sizeof(want)) != NULL, _ret != 0);

	child = fork();
	if (child == 0) {
		for (;;) {
			pause();
		}
	}
	TEST_RES(child, _ret > 0);

	TEST_RES(snprintf(path, sizeof(path), "/proc/%d/cwd", child),
		 _ret > 0 && _ret < (int)sizeof(path));
	TEST_RES(readlink_into(path, got, sizeof(got)), _ret == 0);
	TEST_RES(strcmp(got, want), _ret == 0);

	TEST_SUCC(kill(child, SIGKILL));
	TEST_RES(waitpid(child, NULL, 0), _ret == child);
}
END_TEST()
