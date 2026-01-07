// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <unistd.h>
#include <pthread.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <stdbool.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <libgen.h>
#include <sched.h>
#include <linux/sched.h>
#include <fcntl.h>
#include <signal.h>
#include "../test.h"

struct info {
	bool should_sleep;
};

int exec_child()
{
	char self_path[PATH_MAX];
	ssize_t len = CHECK(
		readlink("/proc/self/exe", self_path, sizeof(self_path) - 1));
	self_path[len] = '\0';

	char *path_copy = strdup(self_path);
	if (path_copy == NULL) {
		return -1;
	}

	char *dir_name = dirname(path_copy);
	if (dir_name == NULL) {
		return -1;
	}

	char exec_path[PATH_MAX];
	char *child_name = "execve_mt_child";
	sprintf(exec_path, "%s/%s", dir_name, child_name);

	FILE *stat;
	int id, flag;

	id = flag = -1;
	CHECK_WITH(stat = fopen("/proc/self/stat", "r"), stat != NULL);
	CHECK_WITH(fscanf(stat, "%d (execve_mt_paren) %n", &id, &flag),
		   _ret == 1);
	CHECK(fclose(stat));
	CHECK_WITH(getpid(), _ret == id && flag != -1);

	id = flag = -1;
	CHECK_WITH(stat = fopen("/proc/thread-self/stat", "r"), stat != NULL);
	CHECK_WITH(fscanf(stat, "%d (execve_mt_paren) %n", &id, &flag),
		   _ret == 1);
	CHECK(fclose(stat));
	CHECK_WITH(syscall(SYS_gettid), _ret == id && flag != -1);

	CHECK(execl(exec_path, child_name, NULL));

	exit(EXIT_FAILURE);
}

void *thread_slave(void *info_)
{
	struct info *info = info_;
	if (info->should_sleep) {
		sleep(1);
	} else {
		exec_child();
	}

	exit(EXIT_FAILURE);
}

#define FILENAME "/tmp/exec_test.stat"

int write_stat(int exit_code, int pipefd)
{
	FILE *file = fopen(FILENAME, "w");
	if (file == NULL) {
		return -1;
	}

	fprintf(file, "%d\n", getpid());
	fprintf(file, "%d\n", exit_code);
	fprintf(file, "%d\n", pipefd);

	CHECK(fclose(file));
	return 0;
}

FN_TEST(exec_in_main_thread)
{
	pid_t pid = TEST_SUCC(fork());

	if (pid == 0) {
		CHECK(write_stat(100, 0));

		struct info info = { .should_sleep = true };

		pthread_t tid1;
		CHECK(pthread_create(&tid1, NULL, &thread_slave, &info));
		pthread_t tid2;
		CHECK(pthread_create(&tid2, NULL, &thread_slave, &info));

		exec_child();
	}

	int status = 0;
	TEST_RES(wait4(pid, &status, 0, NULL),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 100);
}
END_TEST()

FN_TEST(exec_in_slave_thread)
{
	pid_t pid = TEST_SUCC(fork());

	if (pid == 0) {
		CHECK(write_stat(101, 0));

		pthread_t tid1;
		struct info info1 = { .should_sleep = true };
		CHECK(pthread_create(&tid1, NULL, &thread_slave, &info1));
		pthread_t tid2;
		struct info info2 = { .should_sleep = false };
		CHECK(pthread_create(&tid2, NULL, &thread_slave, &info2));

		pthread_join(tid1, NULL);
		pthread_join(tid2, NULL);
		exit(EXIT_FAILURE);
	}

	int status = 0;
	TEST_RES(wait4(pid, &status, 0, NULL),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 101);
}
END_TEST()

pid_t sys_clone3(struct clone_args *args)
{
	return syscall(SYS_clone3, args, sizeof(struct clone_args));
}

FN_TEST(clone_files)
{
	int pipefds[2];
	TEST_SUCC(syscall(SYS_pipe2, pipefds, O_CLOEXEC));

	// Duplicate the pipe fd to a high-value FD to prevent it from being reused.
	int dupped_pipe_fd = 100;
	TEST_SUCC(syscall(SYS_dup3, pipefds[0], dupped_pipe_fd, O_CLOEXEC));
	TEST_SUCC(close(pipefds[0]));

	struct clone_args args = { .flags = CLONE_FILES,
				   .exit_signal = SIGCHLD };
	pid_t pid = TEST_SUCC(sys_clone3(&args));

	if (pid == 0) {
		CHECK(close(pipefds[1]));
		exit(EXIT_SUCCESS);
	}

	int status;
	TEST_RES(wait4(pid, &status, 0, NULL),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	pid = TEST_SUCC(sys_clone3(&args));

	if (pid == 0) {
		CHECK(access("/proc/self/fd/100", F_OK));
		CHECK(access("/proc/thread-self/fd/100", F_OK));
		CHECK(write_stat(102, dupped_pipe_fd));

		struct info info = { .should_sleep = false };
		pthread_t tid1;
		CHECK(pthread_create(&tid1, NULL, &thread_slave, &info));
		pthread_t tid2;
		CHECK(pthread_create(&tid2, NULL, &thread_slave, &info));

		pthread_join(tid1, NULL);
		pthread_join(tid2, NULL);
		exit(EXIT_FAILURE);
	}

	TEST_RES(wait4(pid, &status, 0, NULL),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 102);
	TEST_SUCC(close(dupped_pipe_fd));
	TEST_ERRNO(close(pipefds[1]), EBADF);
}
END_TEST()
