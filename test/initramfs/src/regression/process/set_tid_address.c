// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../common/test.h"

#include <sched.h>
#include <signal.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#define STACK_SIZE (16 * 1024)

static int set_tid_address_child(void *arg)
{
	long tid = syscall(SYS_set_tid_address, arg);
	return tid > 0 ? EXIT_SUCCESS : EXIT_FAILURE;
}

FN_TEST(returns_tid)
{
	long tid = TEST_SUCC(syscall(SYS_set_tid_address, NULL));
	TEST_RES(tid, tid == syscall(SYS_gettid));
}
END_TEST()

FN_TEST(replaces_existing_clear_child_tid)
{
	int *tidptr = TEST_SUCC(mmap(NULL, sizeof(*tidptr) * 2,
				     PROT_READ | PROT_WRITE,
				     MAP_SHARED | MAP_ANONYMOUS, -1, 0));
	int *old_tidptr = &tidptr[0];
	int *new_tidptr = &tidptr[1];
	*old_tidptr = 1;
	*new_tidptr = 1;

	void *stack = TEST_SUCC(mmap(NULL, STACK_SIZE, PROT_READ | PROT_WRITE,
				     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));
	pid_t child = TEST_SUCC(clone(set_tid_address_child,
				      (char *)stack + STACK_SIZE,
				      CLONE_VM | CLONE_CHILD_CLEARTID | SIGCHLD,
				      new_tidptr, NULL, NULL, old_tidptr));

	int status;
	TEST_RES(waitpid(child, &status, 0), _ret == child);
	TEST_RES(status,
		 WIFEXITED(status) && WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_RES(*old_tidptr, *old_tidptr == 1);
	TEST_RES(*new_tidptr, *new_tidptr == 0);
	TEST_SUCC(munmap(stack, STACK_SIZE));
	TEST_SUCC(munmap(tidptr, sizeof(*tidptr) * 2));
}
END_TEST()
