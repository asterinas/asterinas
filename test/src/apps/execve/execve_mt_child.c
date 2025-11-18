// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>
#include <syscall.h>
#include "../test.h"

#define FILENAME "/tmp/exec_test.stat"

FN_SETUP(check_child_stat)
{
	int pid = -1;
	int exit_code = -1;
	int pipefd = -1;

	FILE *file = CHECK_WITH(fopen(FILENAME, "r"), _ret != NULL);
	CHECK_WITH(fscanf(file, "%d %d %d", &pid, &exit_code, &pipefd),
		   _ret == 3);
	CHECK(fclose(file));
	CHECK(unlink(FILENAME));

	CHECK_WITH(getpid(), _ret == pid);
	CHECK_WITH(syscall(SYS_gettid), _ret == pid);

	if (pipefd != 0) {
		CHECK_WITH(access("/proc/self/fd/100", F_OK),
			   _ret == -1 && errno == ENOENT);
		CHECK_WITH(access("/proc/thread-self/fd/100", F_OK),
			   _ret == -1 && errno == ENOENT);
		CHECK_WITH(close(pipefd), _ret == -1 && errno == EBADF);
	}

	FILE *stat;
	int id, flag;

	id = flag = -1;
	CHECK_WITH(stat = fopen("/proc/self/stat", "r"), stat != NULL);
	CHECK_WITH(fscanf(stat, "%d (execve_mt_child) %n", &id, &flag),
		   _ret == 1);
	CHECK(fclose(stat));
	CHECK_WITH(getpid(), _ret == id && flag != -1);

	id = flag = -1;
	CHECK_WITH(stat = fopen("/proc/thread-self/stat", "r"), stat != NULL);
	CHECK_WITH(fscanf(stat, "%d (execve_mt_child) %n", &id, &flag),
		   _ret == 1);
	CHECK(fclose(stat));
	CHECK_WITH(syscall(SYS_gettid), _ret == id && flag != -1);

	exit(exit_code);
}
END_SETUP()
