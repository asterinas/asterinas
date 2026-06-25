// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define THREAD_NAME "TestThread12345"
#define LONG_THREAD_NAME "0123456789abcdeXYZ"
#define TRUNCATED_THREAD_NAME "0123456789abcde\n"

static void format_comm_path(char *path, size_t path_len, pid_t pid, pid_t tid)
{
	CHECK_WITH(snprintf(path, path_len, "/proc/%d/task/%d/comm", pid, tid),
		   _ret > 0 && _ret < path_len);
}

static ssize_t read_comm(const char *path, char *buf, size_t buf_len)
{
	int fd = CHECK(open(path, O_RDONLY));
	ssize_t len = CHECK(read(fd, buf, buf_len - 1));

	CHECK(close(fd));
	buf[len] = '\0';
	return len;
}

static void *set_comm_from_peer(void *arg)
{
	const char *path = arg;
	int fd = open(path, O_WRONLY);
	ssize_t len;

	if (fd < 0) {
		return (void *)(intptr_t)1;
	}

	len = write(fd, THREAD_NAME, strlen(THREAD_NAME));
	if (close(fd) != 0 || len != strlen(THREAD_NAME)) {
		return (void *)(intptr_t)1;
	}

	return NULL;
}

static int create_thread(pthread_t *thread, void *(*start_routine)(void *),
			 void *arg)
{
	int error = pthread_create(thread, NULL, start_routine, arg);

	if (error != 0) {
		errno = error;
		return -1;
	}

	errno = 0;
	return 0;
}

static int join_thread(pthread_t thread, void **retval)
{
	int error = pthread_join(thread, retval);

	if (error != 0) {
		errno = error;
		return -1;
	}

	errno = 0;
	return 0;
}

FN_TEST(comm_contains_thread_name_and_trailing_newline)
{
	char path[128];
	char comm[64];

	format_comm_path(path, sizeof(path), getpid(), syscall(SYS_gettid));
	TEST_SUCC(prctl(PR_SET_NAME, THREAD_NAME));
	read_comm(path, comm, sizeof(comm));

	TEST_RES(strcmp(comm, THREAD_NAME "\n"), _ret == 0);
}
END_TEST()

FN_TEST(comm_can_set_self_thread_name)
{
	char path[128];
	char comm[64];
	int fd;

	format_comm_path(path, sizeof(path), getpid(), syscall(SYS_gettid));
	fd = TEST_SUCC(open(path, O_WRONLY));
	TEST_RES(write(fd, THREAD_NAME, strlen(THREAD_NAME)),
		 _ret == strlen(THREAD_NAME));
	TEST_SUCC(close(fd));
	read_comm(path, comm, sizeof(comm));

	TEST_RES(strcmp(comm, THREAD_NAME "\n"), _ret == 0);
}
END_TEST()

FN_TEST(comm_can_set_peer_thread_name)
{
	char path[128];
	char comm[64];
	pthread_t thread;
	void *result = NULL;

	format_comm_path(path, sizeof(path), getpid(), syscall(SYS_gettid));
	TEST_SUCC(create_thread(&thread, set_comm_from_peer, path));
	TEST_SUCC(join_thread(thread, &result));
	TEST_RES((intptr_t)result, _ret == 0);
	read_comm(path, comm, sizeof(comm));

	TEST_RES(strcmp(comm, THREAD_NAME "\n"), _ret == 0);
}
END_TEST()

FN_TEST(comm_cannot_set_another_process_thread_name)
{
	char path[128];
	pid_t pid;
	int status;

	format_comm_path(path, sizeof(path), getpid(), syscall(SYS_gettid));
	pid = TEST_SUCC(fork());
	if (pid == 0) {
		int fd = open(path, O_WRONLY);

		if (fd < 0) {
			_exit(2);
		}

		errno = 0;
		if (write(fd, "x", 1) == -1 && errno == EINVAL) {
			_exit(0);
		}

		_exit(1);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(comm_len_limited)
{
	char path[128];
	char comm[64];
	int fd;

	format_comm_path(path, sizeof(path), getpid(), syscall(SYS_gettid));
	fd = TEST_SUCC(open(path, O_WRONLY));
	TEST_RES(write(fd, LONG_THREAD_NAME, strlen(LONG_THREAD_NAME)),
		 _ret == strlen(LONG_THREAD_NAME));
	TEST_SUCC(close(fd));
	read_comm(path, comm, sizeof(comm));

	TEST_RES(strcmp(comm, TRUNCATED_THREAD_NAME), _ret == 0);
}
END_TEST()
