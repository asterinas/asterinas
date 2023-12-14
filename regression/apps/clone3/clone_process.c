// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <err.h>
#include <errno.h>
#include <linux/sched.h>
#include <linux/types.h>
#include <sched.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/sysmacros.h>
#include <sys/types.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <unistd.h>

#ifndef CLONE_PIDFD
#define CLONE_PIDFD 0x00001000
#endif

static pid_t sys_clone3(struct clone_args *args)
{
	return syscall(SYS_clone3, args, sizeof(struct clone_args));
}

#define ptr_to_u64(ptr) ((__u64)((uintptr_t)(ptr)))

int main(int argc, char *argv[])
{
	int pidfd = -1;
	pid_t parent_tid = -1, pid = -1;
	struct clone_args args = { 0 };

	args.parent_tid = ptr_to_u64(&parent_tid); /* CLONE_PARENT_SETTID */
	args.flags = CLONE_PARENT_SETTID;
	args.exit_signal = SIGCHLD;

	pid = sys_clone3(&args);
	if (pid < 0) {
		printf("%s - Failed to create new process\n", strerror(errno));
		exit(EXIT_FAILURE);
	}

	if (pid == 0) {
		printf("Child process with pid %d\n", getpid());
		exit(EXIT_SUCCESS);
	}

	printf("Parent process: child's pid %d\n", pid);
	printf("Parent process: child's pid %d in parent_tid\n",
	       *(pid_t *)args.parent_tid);

	waitpid(pid, NULL, 0);

	if (pid != *(pid_t *)args.parent_tid)
		exit(EXIT_FAILURE);

	close(pidfd);

	return 0;
}
