// SPDX-License-Identifier: MPL-2.0

#include "../test.h"

#include <pthread.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/wait.h>

struct exit_info {
	int should_use_exit_group;
	int should_exit_master_first;
};

static void *thread_slave(void *info_)
{
	struct exit_info *info = info_;

	if (info->should_exit_master_first) {
		if (info->should_use_exit_group)
			sleep(3600);
		usleep(200 * 1000);
	}

	if (info->should_use_exit_group)
		syscall(SYS_exit_group, 55);
	else
		syscall(SYS_exit, 66);

	exit(-1);
}

static void *thread_master(void *info_)
{
	struct exit_info *info = info_;
	pthread_t tid;

	CHECK(pthread_create(&tid, NULL, &thread_slave, info));

	if (!info->should_exit_master_first) {
		if (info->should_use_exit_group)
			sleep(3600);
		usleep(200 * 1000);
	}

	if (info->should_use_exit_group)
		syscall(SYS_exit_group, 77);
	else
		syscall(SYS_exit, 88);

	exit(-1);
}

FN_TEST(exit_two_threads)
{
	struct exit_info info;
	int stat;

	info.should_use_exit_group = 1;
	info.should_exit_master_first = 1;
	if (CHECK(fork()) == 0) {
		thread_master(&info);
	}
	TEST_RES(wait(&stat), WIFEXITED(stat) && WEXITSTATUS(stat) == 77);

	info.should_use_exit_group = 1;
	info.should_exit_master_first = 0;
	if (CHECK(fork()) == 0) {
		thread_master(&info);
	}
	TEST_RES(wait(&stat), WIFEXITED(stat) && WEXITSTATUS(stat) == 55);

	info.should_use_exit_group = 0;
	info.should_exit_master_first = 1;
	if (CHECK(fork()) == 0) {
		thread_master(&info);
	}
	TEST_RES(wait(&stat), WIFEXITED(stat) && WEXITSTATUS(stat) == 66);

	info.should_use_exit_group = 0;
	info.should_exit_master_first = 0;
	if (CHECK(fork()) == 0) {
		thread_master(&info);
	}
	TEST_RES(wait(&stat), WIFEXITED(stat) && WEXITSTATUS(stat) == 88);
}
END_TEST()
