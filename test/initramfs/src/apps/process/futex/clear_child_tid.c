// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <errno.h>
#include <linux/futex.h>
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define PAGE_SIZE 4096
#define CHILD_RELEASE_DELAY_US 100000
#define CHILD_WAIT_TIMEOUT_MS 1000
#define OTHER_PROC_TIMEOUT_MS 500
#define CHILD_STACK_SIZE (PAGE_SIZE * 4)

struct futex_wait_result {
	int ret;
	int err;
};

struct futex_wait_args {
	volatile uint32_t *uaddr;
	uint32_t expected;
	int timeout_ms;
	int ready_fd;
	struct futex_wait_result result;
};

static int futex_wait_timed(volatile uint32_t *uaddr, uint32_t expected,
			    int timeout_ms)
{
	struct timespec timeout = {
		.tv_sec = timeout_ms / 1000,
		.tv_nsec = (timeout_ms % 1000) * 1000 * 1000,
	};

	return syscall(SYS_futex, (uint32_t *)uaddr, FUTEX_WAIT, expected,
		       &timeout, NULL, 0);
}

static void *wait_on_futex(void *arg)
{
	struct futex_wait_args *wait_args = arg;
	char ready = 'w';

	if (write(wait_args->ready_fd, &ready, sizeof(ready)) !=
	    sizeof(ready)) {
		wait_args->result.ret = -1;
		wait_args->result.err = errno;
		return NULL;
	}

	errno = 0;
	wait_args->result.ret = futex_wait_timed(
		wait_args->uaddr, wait_args->expected, wait_args->timeout_ms);
	wait_args->result.err = errno;
	return NULL;
}

static void *release_child(void *arg)
{
	int fd = *(int *)arg;

	usleep(CHILD_RELEASE_DELAY_US);
	if (write(fd, "x", 1) != 1) {
		return (void *)1;
	}

	return NULL;
}

static int child_wait_for_release(void *arg)
{
	int fd = *(int *)arg;
	char ch;

	if (read(fd, &ch, sizeof(ch)) != sizeof(ch)) {
		_exit(1);
	}

	return 0;
}

FN_TEST(clear_child_tid_futex_wake_is_address_space_local)
{
	void *mapped = TEST_RES(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
				     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
				_ret != MAP_FAILED);
	volatile uint32_t *futex_word = mapped;
	int other_ready_pipe[2];
	int other_result_pipe[2];
	int child_release_pipe[2];
	int parent_wait_ready_pipe[2];

	*futex_word = 1;

	CHECK(pipe(other_ready_pipe));
	CHECK(pipe(other_result_pipe));
	CHECK(pipe(child_release_pipe));
	CHECK(pipe(parent_wait_ready_pipe));

	int other_process = CHECK(fork());
	if (other_process == 0) {
		struct futex_wait_result result;
		char ready = 'r';

		CHECK(close(other_ready_pipe[0]));
		CHECK(close(other_result_pipe[0]));
		CHECK(close(child_release_pipe[0]));
		CHECK(close(child_release_pipe[1]));
		CHECK(close(parent_wait_ready_pipe[0]));
		CHECK(close(parent_wait_ready_pipe[1]));

		CHECK_WITH(write(other_ready_pipe[1], &ready, sizeof(ready)),
			   _ret == sizeof(ready));

		errno = 0;
		result.ret =
			futex_wait_timed(futex_word, 1, OTHER_PROC_TIMEOUT_MS);
		result.err = errno;

		CHECK_WITH(write(other_result_pipe[1], &result, sizeof(result)),
			   _ret == sizeof(result));
		_exit(0);
	}

	TEST_SUCC(close(other_ready_pipe[1]));
	TEST_SUCC(close(other_result_pipe[1]));

	char ready = 0;
	TEST_RES(read(other_ready_pipe[0], &ready, sizeof(ready)),
		 _ret == sizeof(ready) && ready == 'r');

	void *child_stack =
		TEST_RES(mmap(NULL, CHILD_STACK_SIZE, PROT_READ | PROT_WRITE,
			      MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
			 _ret != MAP_FAILED);
	void *child_stack_top = (char *)child_stack + CHILD_STACK_SIZE;
	int child_pid =
		TEST_RES(clone(child_wait_for_release, child_stack_top,
			       CLONE_VM | CLONE_CHILD_CLEARTID | SIGCHLD,
			       &child_release_pipe[0], NULL, NULL,
			       (uint32_t *)futex_word),
			 _ret > 0);
	TEST_SUCC(close(child_release_pipe[0]));

	*futex_word = child_pid;

	struct futex_wait_args parent_wait_args = {
		.uaddr = futex_word,
		.expected = child_pid,
		.timeout_ms = CHILD_WAIT_TIMEOUT_MS,
		.ready_fd = parent_wait_ready_pipe[1],
		.result = { 0, 0 },
	};
	pthread_t parent_wait_thread;
	TEST_RES(pthread_create(&parent_wait_thread, NULL, wait_on_futex,
				&parent_wait_args),
		 _ret == 0);

	char waiter_ready = 0;
	TEST_RES(read(parent_wait_ready_pipe[0], &waiter_ready,
		      sizeof(waiter_ready)),
		 _ret == sizeof(waiter_ready) && waiter_ready == 'w');

	pthread_t release_thread;
	TEST_RES(pthread_create(&release_thread, NULL, release_child,
				&child_release_pipe[1]),
		 _ret == 0);

	TEST_RES(pthread_join(parent_wait_thread, NULL), _ret == 0);
	TEST_RES(pthread_join(release_thread, NULL), _ret == 0);

	struct futex_wait_result other_result = { 0, 0 };
	TEST_RES(read(other_result_pipe[0], &other_result,
		      sizeof(other_result)),
		 _ret == sizeof(other_result));
	TEST_RES(waitpid(other_process, NULL, 0), _ret == other_process);
	TEST_RES(parent_wait_args.result.ret,
		 _ret == 0 && parent_wait_args.result.err == 0 &&
			 *futex_word == 0);
	TEST_RES(other_result.ret, _ret == -1 && other_result.err == ETIMEDOUT);
	TEST_SUCC(close(other_ready_pipe[0]));
	TEST_SUCC(close(other_result_pipe[0]));
	TEST_SUCC(close(child_release_pipe[1]));
	TEST_SUCC(close(parent_wait_ready_pipe[0]));
	TEST_SUCC(close(parent_wait_ready_pipe[1]));
	TEST_SUCC(munmap(child_stack, CHILD_STACK_SIZE));
	TEST_SUCC(munmap((void *)futex_word, PAGE_SIZE));
}
END_TEST()
