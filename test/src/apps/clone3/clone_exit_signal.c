// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <err.h>
#include <linux/sched.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <sys/wait.h>

static pid_t sys_clone3(struct clone_args *args)
{
	return syscall(SYS_clone3, args, sizeof(struct clone_args));
}

int child_exit_recv = 0;

void sig_handler(int signal)
{
	printf("Received child exit signal\n");
	child_exit_recv++;
}

int main(int argc, char *argv[])
{
	pid_t pid;
	struct clone_args args = {
		.exit_signal = SIGUSR2,
	};

	signal(SIGUSR2, sig_handler);

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
	if (waitpid(pid, NULL, __WALL) < 0)
		err(EXIT_FAILURE, "cannot wait child process");

	if (child_exit_recv != 1)
		errx(EXIT_FAILURE, "did not receive exit signal from child");

	return 0;
}
