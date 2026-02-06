// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <unistd.h>
#include <fcntl.h>
#include <wait.h>

#include "../../common/test.h"

FN_TEST(negative_cache_pid)
{
	pid_t pid, pid2;
	char path[20];
	int i, status;

	pid = TEST_SUCC(getpid());

	// These paths may not yet exist, but we cannot cache negative results here.
	for (i = 0; i < 100; ++i) {
		snprintf(path, sizeof(path), "/proc/%d", pid + i);
		(void)!access(path, F_OK);
	}

	pid2 = TEST_SUCC(fork());
	if (pid2 == 0) {
		CHECK(access("/proc/self/", F_OK));

		usleep(200 * 1000);
		exit(EXIT_SUCCESS);
	}

	snprintf(path, sizeof(path), "/proc/%d", pid2);
	TEST_SUCC(access(path, F_OK));

	TEST_RES(wait(&status), _ret == pid2 && WIFEXITED(status) &&
					WEXITSTATUS(status) == EXIT_SUCCESS);

	snprintf(path, sizeof(path), "/proc/%d", pid2);
	TEST_ERRNO(access(path, F_OK), ENOENT);
}
END_TEST()

FN_TEST(negative_cache_fd)
{
	TEST_ERRNO(access("/proc/self/fdinfo/100", F_OK), ENOENT);

	TEST_SUCC(dup2(0, 100));
	TEST_SUCC(access("/proc/self/fdinfo/100", F_OK));

	TEST_SUCC(close(100));
	TEST_ERRNO(access("/proc/self/fdinfo/100", F_OK), ENOENT);
}
END_TEST()
