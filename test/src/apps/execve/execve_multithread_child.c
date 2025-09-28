// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>
#include <syscall.h>
#include <signal.h>
#include "../test.h"

const char *filename = "/tmp/exec_test.stat";

int read_line_as_number(FILE *file)
{
	char line[128];
	fgets(line, sizeof(line), file);

	char *pos = strchr(line, '\n');
	if (pos != NULL) {
		*pos = '\0';
	}

	return (int)strtol(line, NULL, 10);
}

FN_SETUP(check_child_stat)
{
	FILE *file = CHECK(fopen(filename, "r"));

	int pid = read_line_as_number(file);
	CHECK_WITH(getpid(), _ret == pid);
	CHECK_WITH(syscall(SYS_gettid), _ret == pid);

	int exit_code = read_line_as_number(file);
	if (exit_code == 103) {
		struct sigaction sa;
		CHECK_WITH(sigaction(SIGIO, NULL, &sa),
			   sa.sa_handler == SIG_DFL);
		CHECK_WITH(sigaction(SIGINT, NULL, &sa),
			   sa.sa_handler == SIG_DFL);
	}

	int pipefd = read_line_as_number(file);
	if (pipefd != 0) {
		CHECK_WITH(close(pipefd), errno == EBADF);
	}

	CHECK(fclose(file));

	CHECK(unlink(filename));

	exit(exit_code);
}
END_SETUP()