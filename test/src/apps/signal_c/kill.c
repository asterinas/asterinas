// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <pthread.h>
#include <signal.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/wait.h>

#include "../test.h"

FN_SETUP(setpgrp)
{
	CHECK(setpgrp());
}
END_SETUP()

FN_TEST(kill_dead_process)
{
	pid_t ppid, cpid;
	int status;

	ppid = TEST_SUCC(getpid());
	cpid = TEST_SUCC(fork());
	if (cpid == 0) {
		exit(EXIT_SUCCESS);
	}
	usleep(200 * 1000);

	TEST_SUCC(kill(ppid, SIGCHLD));
	TEST_SUCC(kill(-ppid, SIGCHLD));

	// Killing dead processes will succeed.
	TEST_SUCC(kill(cpid, SIGCHLD));
	TEST_ERRNO(kill(-cpid, SIGCHLD), ESRCH);

	TEST_RES(wait(&status),
		 _ret == cpid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(kill_dead_group)
{
	pid_t ppid, cpid;
	int status;

	ppid = TEST_SUCC(getpid());
	cpid = TEST_SUCC(fork());
	if (cpid == 0) {
		CHECK(setpgrp());
		exit(EXIT_SUCCESS);
	}
	usleep(200 * 1000);

	TEST_SUCC(kill(ppid, SIGCHLD));
	TEST_SUCC(kill(-ppid, SIGCHLD));

	// Killing dead process groups will succeed.
	TEST_SUCC(kill(cpid, SIGCHLD));
	TEST_SUCC(kill(-cpid, SIGCHLD));

	TEST_RES(wait(&status),
		 _ret == cpid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

static void *background_thread(void *data)
{
	sleep(3600);
	return NULL;
}

FN_TEST(kill_dead_thread)
{
	pid_t cpid;
	int status;

	cpid = TEST_SUCC(fork());
	if (cpid == 0) {
		pthread_t tid;
		CHECK(pthread_create(&tid, NULL, background_thread, NULL));
		syscall(SYS_exit, 0);
	}
	usleep(200 * 1000);

	// Killing dead threads will succeed.
	TEST_SUCC(kill(cpid, SIGCHLD));
	TEST_SUCC(tgkill(cpid, cpid, SIGCHLD));

	TEST_SUCC(kill(cpid, SIGKILL));
	TEST_RES(wait(&status), _ret == cpid && WIFSIGNALED(status) &&
					WTERMSIG(status) == SIGKILL);
}
END_TEST()
