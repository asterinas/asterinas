// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"

#include <sys/mman.h>
#include <sys/ptrace.h>
#include <time.h>

struct sibling_state {
	int tid;
	int ready;
	int old_tid;
};

static void *wait_for_group_exit(void *arg)
{
	struct sibling_state *state = arg;
	__atomic_store_n(&state->tid, syscall(SYS_gettid), __ATOMIC_RELEASE);
	state->old_tid = state->tid;
	CHECK_WITH(syscall(SYS_set_tid_address, &state->old_tid),
		   _ret == state->tid);
	CHECK(syscall(SYS_set_tid_address, &state->tid));
	__atomic_store_n(&state->ready, 1, __ATOMIC_RELEASE);
	for (;;)
		pause();
	return NULL;
}

/* A stopped exiting thread must not defer the killing of its siblings. */
static int siblings_exit_before_ptrace_stop(int use_seccomp)
{
	struct sibling_state *state =
		CHECK_WITH(mmap(NULL, sizeof(*state), PROT_READ | PROT_WRITE,
				MAP_SHARED | MAP_ANONYMOUS, -1, 0),
			   _ret != MAP_FAILED);
	state->tid = -1;
	state->ready = 0;
	fflush(NULL);
	pid_t child = CHECK(fork());
	if (child == 0) {
		no_new_privs();
		deny_syscall(SYS_getppid, SECCOMP_RET_KILL_PROCESS);
		pthread_t sibling;
		CHECK_WITH(pthread_create(&sibling, NULL, wait_for_group_exit,
					  state),
			   _ret == 0);
		while (!__atomic_load_n(&state->ready, __ATOMIC_ACQUIRE))
			sched_yield();
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		if (use_seccomp)
			syscall(SYS_getppid);
		else
			syscall(SYS_exit_group, 37);
		_exit(1);
	}

	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status) &&
			   WSTOPSIG(status) == SIGSTOP);
	CHECK(ptrace(PTRACE_SETOPTIONS, child, 0, PTRACE_O_TRACEEXIT));
	CHECK(ptrace(PTRACE_CONT, child, 0, 0));
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status) &&
			   (unsigned int)status >> 16 == PTRACE_EVENT_EXIT);
	unsigned long event_status;
	CHECK(ptrace(PTRACE_GETEVENTMSG, child, 0, &event_status));
	int failed = use_seccomp ? (event_status & 0x7f) != SIGSYS :
				   event_status != (37 << 8);
	struct timespec delay = { .tv_nsec = 1000000 };
	for (int i = 0;
	     i < 2000 && __atomic_load_n(&state->tid, __ATOMIC_ACQUIRE) != 0;
	     ++i)
		nanosleep(&delay, NULL);
	failed |= __atomic_load_n(&state->tid, __ATOMIC_ACQUIRE) != 0;
	/* Replacing clear_child_tid must not clear the old address. */
	failed |= state->old_tid <= 0;
	/* Resume and reap even if the timing assertion failed. */
	CHECK(ptrace(PTRACE_CONT, child, 0, 0));
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	failed |= use_seccomp ?
			  !(WIFSIGNALED(status) && WTERMSIG(status) == SIGSYS) :
			  !(WIFEXITED(status) && WEXITSTATUS(status) == 37);
	CHECK(munmap(state, sizeof(*state)));
	return failed;
}

FN_TEST(group_exit_ptrace_order)
{
	TEST_RES(siblings_exit_before_ptrace_stop(1), _ret == 0);
	TEST_RES(siblings_exit_before_ptrace_stop(0), _ret == 0);
}
END_TEST()
