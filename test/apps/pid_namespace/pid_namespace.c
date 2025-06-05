// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <errno.h>

#define STACK_SIZE (1024 * 1024)

static char child_stack[STACK_SIZE];

#define CHECK(cond)                                                     \
	if (!(cond)) {                                                  \
		fprintf(stderr, "fatal error: `" #cond "` is false\n"); \
		exit(EXIT_FAILURE);                                     \
	}

int child_in_new_pid_ns(void *arg)
{
	CHECK(getpid() == 1);
	CHECK(getppid() == 0);

	exit(0);
}

int sleep_1s_child(void *args)
{
	sleep(1);
	exit(0);
}

void basic_test()
{
	int parent_pid = getpid();

	// ClONE_NEWPID cannot be used toghther with CLONE_THREAD
	clone(sleep_1s_child, child_stack + STACK_SIZE,
	      SIGCHLD | CLONE_NEWPID | CLONE_THREAD, NULL);
	CHECK(errno == EINVAL);

	// With CLONE_NEWPID, the multiple cloned processes are actually in different PID namespaces.
	int child_pid = clone(sleep_1s_child, child_stack + STACK_SIZE,
			      SIGCHLD | CLONE_NEWPID, NULL);
	CHECK(child_pid > 0);
	CHECK(child_pid > parent_pid);

	int child_pid2 = clone(child_in_new_pid_ns, child_stack + STACK_SIZE,
			       SIGCHLD | CLONE_NEWPID, NULL);
	CHECK(child_pid2 > 0);

	// Wait for the child processes to terminate
	int status = 0;

	waitpid(child_pid2, &status, 0);
	CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 0);

	waitpid(child_pid, &status, 0);
	CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 0);
}

int nested_child(void *args)
{
	CHECK(getpid() == 1);
	CHECK(getppid() == 0);

	int child_pid = clone(child_in_new_pid_ns, child_stack + STACK_SIZE,
			      SIGCHLD | CLONE_NEWPID, NULL);
	CHECK(child_pid > 0);

	int status = 0;
	waitpid(child_pid, &status, 0);
	CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 0);

	exit(0);
}

void test_nested()
{
	int child_pid = clone(nested_child, child_stack + STACK_SIZE,
			      SIGCHLD | CLONE_NEWPID, NULL);
	CHECK(child_pid > 0);

	int status = 0;
	waitpid(child_pid, &status, 0);
	CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 0);
}

int sleep_2s_child(void *args)
{
	sleep(2);
	exit(0);
}

int pgrp_and_session_child(void *args)
{
	sleep(1);

	int child_pid = getpid();
	int child_pgid = getpgid(child_pid);
	int child_sid = getsid(child_pid);
	CHECK(child_pid == 1);
	CHECK(child_pgid == 0);
	CHECK(child_sid == 0);

	setsid();

	int new_child_pgid = getpgid(child_pid);
	int new_child_sid = getsid(child_pid);

	CHECK(new_child_pgid == child_pid);
	CHECK(new_child_sid == child_pid);

	exit(0);
}

void test_pgrp_and_session()
{
	int parent_pid = getpid();
	int parent_pgid = getpgid(parent_pid);
	int parent_sid = getsid(parent_pid);

	int child_pid = clone(pgrp_and_session_child, child_stack + STACK_SIZE,
			      SIGCHLD | CLONE_NEWPID, NULL);
	CHECK(child_pid > 0);

	// The child process is in the same process group and session as the parent.
	int child_pgid = getpgid(child_pid);
	int child_sid = getsid(child_pid);
	CHECK(parent_pgid == child_pgid);
	CHECK(parent_sid == child_sid);

	// Wait until child process is moved to new session.
	sleep(2);

	int new_child_pgid = getpgid(child_pid);
	int new_child_sid = getsid(child_pid);

	// The new process group and session is visible in parent namespace.
	CHECK(new_child_pgid == child_pid);
	CHECK(new_child_sid == child_pid);

	int status = 0;
	waitpid(child_pid, &status, 0);
	CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 0);
}

void test_unshare()
{
	// Once the PID namespace is unshared,
	// all following cloned process will be in a same new PID namespace.
	unshare(CLONE_NEWPID);

	// Call unshare again which will fail.
	CHECK(unshare(CLONE_NEWPID) < 0 && errno == EINVAL);

	// After calling unshare, clone with `CLONE_NEW_PID` will fail.
	int error = clone(sleep_1s_child, child_stack + STACK_SIZE,
			  SIGCHLD | CLONE_NEWPID, NULL);
	CHECK(error < 0 && errno == EINVAL);

	int child_pid1 =
		clone(sleep_1s_child, child_stack + STACK_SIZE, SIGCHLD, NULL);
	int child_pid2 =
		clone(sleep_2s_child, child_stack + STACK_SIZE, SIGCHLD, NULL);
	CHECK(child_pid1 > 0);
	CHECK(child_pid2 > 0);

	// Wait for the child process to terminate
	int status = 0;

	// If the "init" process of a PID namespace terminates, the kernel
	// terminates all of the processes in the namespace via a SIGKILL signal.
	waitpid(child_pid2, &status, 0);
	CHECK(WIFSIGNALED(status) && WTERMSIG(status) == SIGKILL);

	waitpid(child_pid1, &status, 0);
	CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 0);

	// Since init process is terminated, the following clone will all fail.
	clone(sleep_1s_child, child_stack + STACK_SIZE, SIGCHLD, NULL);
	CHECK(errno == ENOMEM);
}

int main()
{
	printf("Running `basic_test`......\n");
	basic_test();
	printf("Running `test_nested`......\n");
	test_nested();
	printf("Running `test_pgrp_and_session`......\n");
	test_pgrp_and_session();
	printf("Running `test_unshare`......\n");
	test_unshare();
	printf("All test passed.\n");

	return 0;
}
