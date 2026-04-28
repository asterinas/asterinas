// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

struct tid_test_context {
	int ready_pipe[2];
	int exit_pipe[2];
	pid_t tid;
};

static void print_command(char *command, size_t command_len, const char *format,
			  int id)
{
	CHECK_WITH(snprintf(command, command_len, format, id),
		   _ret > 0 && _ret < command_len);
}

static void *thread_fn(void *arg)
{
	struct tid_test_context *context = arg;
	char ch;

	context->tid = CHECK(syscall(SYS_gettid));
	CHECK_WITH(write(context->ready_pipe[1], "R", 1), _ret == 1);
	CHECK_WITH(read(context->exit_pipe[0], &ch, 1), _ret == 1 && ch == 'X');

	return NULL;
}

FN_TEST(proc_root_tid_entry)
{
	struct tid_test_context context = {
		.ready_pipe = { -1, -1 },
		.exit_pipe = { -1, -1 },
		.tid = -1,
	};
	char command[128];
	char ch;
	struct stat first;
	struct stat second;
	pthread_t thread;
	pid_t pid = TEST_SUCC(getpid());

	/* Keep a non-leader thread alive while probing its procfs entry. */
	TEST_SUCC(pipe(context.ready_pipe));
	TEST_SUCC(pipe(context.exit_pipe));
	TEST_SUCC(pthread_create(&thread, NULL, thread_fn, &context));
	TEST_RES(read(context.ready_pipe[0], &ch, 1),
		 _ret == 1 && ch == 'R' && context.tid > 0 &&
			 context.tid != pid);

	/* /proc/<tid> exists while the non-leader thread is alive. */
	print_command(command, sizeof(command), "ls /proc/%d > /dev/null",
		      context.tid);
	TEST_RES(system(command), _ret == 0);

	/* Repeated lookups of the live /proc/<tid> entry resolve to one inode. */
	print_command(command, sizeof(command), "/proc/%d", context.tid);
	TEST_SUCC(stat(command, &first));
	TEST_RES(stat(command, &second), first.st_ino == second.st_ino);

	/* The non-leader tid is directly lookable but not listed in /proc. */
	print_command(command, sizeof(command),
		      "ls /proc -al | grep ' %d$' > /dev/null", context.tid);
	TEST_RES(system(command), _ret != 0);

	/* The process leader remains listed in /proc. */
	print_command(command, sizeof(command),
		      "ls /proc -al | grep ' %d$' > /dev/null", pid);
	TEST_RES(system(command), _ret == 0);

	/* Release the thread after all /proc/<tid> observations are done. */
	TEST_RES(write(context.exit_pipe[1], "X", 1), _ret == 1);
	TEST_SUCC(pthread_join(thread, NULL));
	TEST_SUCC(close(context.ready_pipe[0]));
	TEST_SUCC(close(context.ready_pipe[1]));
	TEST_SUCC(close(context.exit_pipe[0]));
	TEST_SUCC(close(context.exit_pipe[1]));
}
END_TEST()
