// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <err.h>
#include <linux/sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <sys/wait.h>

static pid_t sys_clone3(struct clone_args *args)
{
	return syscall(SYS_clone3, args, sizeof(struct clone_args));
}

int main(int argc, char *argv[])
{
	pid_t pid;
	struct clone_args args = {
		.exit_signal = 0,
	};

	pid = sys_clone3(&args);
	if (pid < 0)
		err(EXIT_FAILURE, "Failed to create new process");

	if (pid == 0) {
		printf("Child process with pid %d\n", getpid());
		exit(EXIT_SUCCESS);
	}

	/*
	 * From the clone(2) manual:
	 * If [the exit signal] is specified as anything other than SIGCHLD,
	 * then the parent process must specify the __WALL or __WCLONE
	 * options when waiting for the child with wait(2).
	 */
	waitpid(pid, NULL, __WALL);

	/* We should have gotten this far without receiving any signals */
	return 0;
}
