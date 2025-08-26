// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sys/ptrace.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <string.h>
#include <unistd.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>

void print_tracerpid(pid_t pid)
{
	char path[256];
	snprintf(path, sizeof(path), "/proc/%d/status", pid);

	FILE *f = fopen(path, "r");
	if (!f) {
		perror("fopen");
		return;
	}

	char line[256];
	while (fgets(line, sizeof(line), f)) {
		if (strncmp(line, "TracerPid:", 10) == 0) {
			printf("[pid=%d] %s", pid, line);
			break;
		}
	}

	fclose(f);
}

int main()
{
	pid_t child = fork();
	if (child < 0) {
		perror("fork");
		exit(1);
	}

	if (child == 0) {
		if (ptrace(PTRACE_TRACEME, 0, NULL, NULL) == -1) {
			perror("ptrace TRACEME");
			exit(1);
		}

		print_tracerpid(getpid());

		printf("[child] sending SIGSTOP to self\n");
		raise(SIGSTOP);

		printf("[child] resumed, exiting\n");
		exit(0);
	} else {
		printf("[parent] parent pid: %d, child pid: %d\n", getpid(),
		       child);
		int status;
		waitpid(child, &status, 0);
		if (WIFSTOPPED(status)) {
			printf("[parent] child stopped by signal %d\n",
			       WSTOPSIG(status));
		}

		ptrace(PTRACE_CONT, child, NULL, NULL);
	}

	return 0;
}
