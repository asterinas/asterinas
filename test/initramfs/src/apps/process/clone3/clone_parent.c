// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <linux/sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>
#include <sched.h>
#include <fcntl.h>
#include "../../common/test.h"

pid_t sys_clone3(struct clone_args *args)
{
	return syscall(SYS_clone3, args, sizeof(struct clone_args));
}

FN_TEST(clone_process)
{
	pid_t pid = getpid();
	TEST_ERRNO(wait4(-1, NULL, 0, NULL), ECHILD);

	pid_t child_pid = TEST_SUCC(fork());

	if (child_pid == 0) {
		// Child process
		CHECK_WITH(getppid(), _ret == pid);

		// CLONE_PARENT cannot be used if exit_signal is not 0
		struct clone_args args = { .flags = CLONE_PARENT,
					   .exit_signal = SIGCHLD };
		CHECK_WITH(sys_clone3(&args), errno == EINVAL);

		args.exit_signal = 0;

		// Check process group and session
		pid_t sid = CHECK(setsid());
		pid_t pgid = CHECK(getpgrp());
		pid_t grandchild_pid = CHECK(sys_clone3(&args));
		if (grandchild_pid == 0) {
			// Grandchild process 1
			CHECK_WITH(getppid(), _ret == pid);
			CHECK_WITH(getpgrp(), _ret == pgid);
			CHECK_WITH(getsid(0), _ret == sid);
			exit(EXIT_SUCCESS);
		}

		// Check files
		int pipefds[2];
		CHECK(pipe(pipefds));
		grandchild_pid = CHECK(sys_clone3(&args));
		if (grandchild_pid == 0) {
			// Grandchild process 2
			CHECK(close(pipefds[0]));
			char buf[1] = { 'a' };
			CHECK_WITH(write(pipefds[1], buf, 1), _ret == 1);
			exit(EXIT_SUCCESS);
		}

		CHECK(close(pipefds[1]));
		char buf[1];
		CHECK_WITH(read(pipefds[0], buf, 1),
			   _ret == 1 && buf[0] == 'a');

		// Check child subreaper
		CHECK(prctl(PR_SET_CHILD_SUBREAPER, 1));
		int subreaper = 0;
		CHECK_WITH(prctl(PR_GET_CHILD_SUBREAPER, &subreaper),
			   subreaper == 1);
		grandchild_pid = CHECK(sys_clone3(&args));
		if (grandchild_pid == 0) {
			// Grandchild process 3
			CHECK_WITH(prctl(PR_GET_CHILD_SUBREAPER, &subreaper),
				   subreaper == 0);
			exit(EXIT_SUCCESS);
		}

		CHECK_WITH(wait4(-1, NULL, 0, NULL), errno == ECHILD);
		exit(EXIT_SUCCESS);
	}

	// Parent process
	int status = 0;
	TEST_RES(wait4(-1, &status, 0, NULL),
		 WIFEXITED(status) && WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_RES(wait4(-1, &status, 0, NULL),
		 WIFEXITED(status) && WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_RES(wait4(-1, &status, 0, NULL),
		 WIFEXITED(status) && WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_RES(wait4(-1, &status, 0, NULL),
		 WIFEXITED(status) && WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_ERRNO(wait4(-1, NULL, 0, NULL), ECHILD);
}
END_TEST()

int pipefds[2];
// A stack for the new thread
#define STACK_SIZE (1024 * 1024) // 1MB stack
char child_stack[STACK_SIZE];

int child_function(void *arg)
{
	char buf[1] = { 'a' };
	CHECK(write(pipefds[1], buf, 1));
	return 0;
}

FN_TEST(clone_thread)
{
	char buf[1] = { 0 };
	TEST_SUCC((pipe(pipefds)));

	// Clone
	TEST_SUCC(clone(child_function, child_stack + STACK_SIZE,
			CLONE_PARENT | CLONE_THREAD | CLONE_VM | CLONE_SIGHAND,
			NULL));
	TEST_RES(read(pipefds[0], buf, 1), buf[0] == 'a');

	// Clone
	TEST_SUCC(clone(child_function, child_stack + STACK_SIZE,
			CLONE_PARENT | CLONE_THREAD | CLONE_VM | CLONE_SIGHAND |
				SIGCHLD,
			NULL));
	TEST_RES(read(pipefds[0], buf, 1), buf[0] == 'a');

	// Clone3
	struct clone_args args = { .flags = CLONE_PARENT | CLONE_THREAD |
					    CLONE_VM | CLONE_SIGHAND,
				   .exit_signal = SIGCHLD };
	TEST_ERRNO(sys_clone3(&args), EINVAL);

	// Clone3
	args.flags = CLONE_THREAD | CLONE_VM | CLONE_SIGHAND;
	TEST_ERRNO(sys_clone3(&args), EINVAL);
}
END_TEST()

// The init process in each PID namespaces can not specify the CLONE_PARENT flags.
// The whole test case is commented out since Asterinas does not support PID namespace.
// FN_TEST(clone_init_process)
// {
// 	struct clone_args args = { .flags = CLONE_NEWPID };
//
// 	int child_pid = TEST_SUCC(sys_clone3(&args));
//
// 	if (child_pid == 0) {
// 		// Child process
// 		CHECK_WITH(getpid(), _ret == 1);
//
// 		args.flags = CLONE_PARENT;
// 		CHECK_WITH(sys_clone3(&args), errno == EINVAL);
//
// 		args.flags = CLONE_PARENT | CLONE_THREAD | CLONE_VM |
// 			     CLONE_SIGHAND;
// 		CHECK_WITH(sys_clone3(&args), errno == EINVAL);
//
// 		exit(EXIT_SUCCESS);
// 	}
//
// 	int status = 0;
// 	TEST_RES(wait4(-1, &status, 0, NULL),
// 		 _ret == child_pid && WIFEXITED(status) &&
// 			 WEXITSTATUS(status) == EXIT_SUCCESS);
// 	TEST_ERRNO(wait4(-1, NULL, 0, NULL), ECHILD);
// }
// END_TEST()