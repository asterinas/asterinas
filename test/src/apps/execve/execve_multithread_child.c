// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>
#include <syscall.h>
#include <signal.h>
#include "../test.h"

#define FILENAME "/tmp/exec_test.stat"

FN_SETUP(check_child_stat)
{
	FILE *file = CHECK(fopen(FILENAME, "r"));

	int pid;
	int exit_code;
	int pipefd;

	fscanf(file, "%d %d %d", &pid, &exit_code, &pipefd);
	CHECK_WITH(getpid(), _ret == pid);
	CHECK_WITH(syscall(SYS_gettid), _ret == pid);

	if (pipefd != 0) {
		CHECK_WITH(close(pipefd), errno == EBADF);
	}

	CHECK(fclose(file));

	CHECK(unlink(FILENAME));

	exit(exit_code);
}
END_SETUP()