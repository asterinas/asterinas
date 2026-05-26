// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define THREAD_NAME "fork_comm"

static int check_proc_stat_comm(const char *expected_comm)
{
	char expected_prefix[64];
	char stat[128];
	FILE *file;
	int pid;

	file = CHECK_WITH(fopen("/proc/self/stat", "r"), _ret != NULL);
	CHECK_WITH(fgets(stat, sizeof(stat), file), _ret != NULL);
	CHECK_WITH(fclose(file), _ret == 0);

	pid = getpid();
	snprintf(expected_prefix, sizeof(expected_prefix), "%d (%s) ", pid,
		 expected_comm);

	return strncmp(stat, expected_prefix, strlen(expected_prefix)) != 0;
}

FN_TEST(fork_inherits_thread_name)
{
	int status;
	pid_t pid;

	TEST_SUCC(prctl(PR_SET_NAME, THREAD_NAME));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK_WITH(check_proc_stat_comm(THREAD_NAME), _ret == 0);
		_exit(0);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()
