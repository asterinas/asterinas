// SPDX-License-Identifier: MPL-2.0

#include "../test.h"

#include <unistd.h>
#include <pthread.h>
#include <sys/syscall.h>
#include <sys/wait.h>

#define EXIT_PARENT_FIRST ((void *)1)
#define EXIT_CHILD_FIRST ((void *)2)

// TODO: Use `system` directly after we implement `vfork`.
static int mini_system(const char *cmd)
{
	int pid;
	int stat;

	pid = CHECK(fork());

	if (pid == 0) {
		CHECK(execlp("sh", "sh", "-c", cmd, NULL));
		exit(-1);
	}

	CHECK_WITH(waitpid(pid, &stat, 0), _ret == pid && WIFEXITED(stat));

	return WEXITSTATUS(stat);
}

static void *thread_slave(void *arg)
{
	if (arg == EXIT_CHILD_FIRST) {
		CHECK_WITH(
			mini_system(
				"cat /proc/$PPID/status | grep '^Threads:\t2$'"),
			_ret == 0);

		syscall(SYS_exit, 0);
	} else {
		// When the main thread exits, it becomes a zombie, so we still
		// have two threads.
		usleep(200 * 1000);
		CHECK_WITH(
			mini_system(
				"cat /proc/$PPID/status | grep '^Threads:\t2$'"),
			_ret == 0);

		syscall(SYS_exit, 0);
	}

	exit(-1);
}

static void thread_master(void *arg)
{
	pthread_t tid;

	CHECK(pthread_create(&tid, NULL, &thread_slave, arg));

	if (arg == EXIT_PARENT_FIRST) {
		CHECK_WITH(
			mini_system(
				"cat /proc/$PPID/status | grep '^Threads:\t2$'"),
			_ret == 0);

		syscall(SYS_exit, 0);
	} else {
		// When a non-main thread exits, its resource is automatically
		// freed, so we only have one thread.
		usleep(200 * 1000);
		CHECK_WITH(
			mini_system(
				"cat /proc/$PPID/status | grep '^Threads:\t1$'"),
			_ret == 0);

		syscall(SYS_exit, 0);
	}

	exit(-1);
}

FN_TEST(exit_procfs)
{
	int stat;

	if (CHECK(fork()) == 0) {
		thread_master(EXIT_CHILD_FIRST);
	}
	TEST_RES(wait(&stat), WIFEXITED(stat) && WEXITSTATUS(stat) == 0);

	if (CHECK(fork()) == 0) {
		thread_master(EXIT_PARENT_FIRST);
	}
	TEST_RES(wait(&stat), WIFEXITED(stat) && WEXITSTATUS(stat) == 0);
}
END_TEST()
