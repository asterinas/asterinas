// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <unistd.h>
#include <sched.h>
#include <sys/stat.h>
#include <pthread.h>
#include <fcntl.h>

#include "../../common/test.h"

FN_TEST(invalid_flags)
{
	TEST_ERRNO(unshare(CLONE_CHILD_CLEARTID), EINVAL);
	TEST_ERRNO(unshare(CLONE_CHILD_SETTID), EINVAL);
	TEST_ERRNO(unshare(CLONE_DETACHED), EINVAL);
	TEST_ERRNO(unshare(CLONE_IO), EINVAL);
	TEST_ERRNO(unshare(CLONE_PARENT), EINVAL);
	TEST_ERRNO(unshare(CLONE_PARENT_SETTID), EINVAL);
	TEST_ERRNO(unshare(CLONE_PIDFD), EINVAL);
	TEST_ERRNO(unshare(CLONE_PTRACE), EINVAL);
	TEST_ERRNO(unshare(CLONE_SETTLS), EINVAL);
	TEST_ERRNO(unshare(CLONE_UNTRACED), EINVAL);
	TEST_ERRNO(unshare(CLONE_VFORK), EINVAL);
}
END_TEST()

void *sleep_1s_thread(void *arg)
{
	sleep(1);

	return NULL;
}

FN_TEST(single_thread_flags)
{
	TEST_SUCC(unshare(CLONE_VM | CLONE_SIGHAND | CLONE_THREAD));

	pthread_t thread_id;
	TEST_SUCC(pthread_create(&thread_id, NULL, sleep_1s_thread, NULL));

	TEST_ERRNO(unshare(CLONE_VM), EINVAL);
	TEST_ERRNO(unshare(CLONE_SIGHAND), EINVAL);
	TEST_ERRNO(unshare(CLONE_THREAD), EINVAL);

	TEST_SUCC(pthread_join(thread_id, NULL));

	TEST_SUCC(unshare(CLONE_VM));
	TEST_SUCC(unshare(CLONE_SIGHAND));
	TEST_SUCC(unshare(CLONE_THREAD));
}
END_TEST()

void *unshare_files_thread(void *arg)
{
	int test_fd = (int)(intptr_t)arg;

	CHECK(unshare(CLONE_FILES));
	CHECK(close(test_fd));

	return NULL;
}

FN_TEST(unshare_files)
{
	struct stat old_stdin_stat, old_stdout_stat, old_stderr_stat;
	struct stat new_stdin_stat, new_stdout_stat, new_stderr_stat;

	TEST_SUCC(fstat(STDIN_FILENO, &old_stdin_stat));
	TEST_SUCC(fstat(STDOUT_FILENO, &old_stdout_stat));
	TEST_SUCC(fstat(STDERR_FILENO, &old_stderr_stat));

	TEST_SUCC(unshare(CLONE_FILES));

	TEST_RES(fstat(STDIN_FILENO, &new_stdin_stat),
		 old_stdin_stat.st_ino == new_stdin_stat.st_ino);
	TEST_RES(fstat(STDOUT_FILENO, &new_stdout_stat),
		 old_stdout_stat.st_ino == new_stdout_stat.st_ino);
	TEST_RES(fstat(STDERR_FILENO, &new_stderr_stat),
		 old_stderr_stat.st_ino == new_stderr_stat.st_ino);

	const char *TEST_FILENAME = "/tmp/unshare_files_test.txt";
	pthread_t thread_id;
	struct stat stat1, stat2;
	int test_fd;

	test_fd = TEST_SUCC(
		open(TEST_FILENAME, O_CREAT | O_RDWR | O_TRUNC, 0644));
	TEST_SUCC(fstat(test_fd, &stat1));

	TEST_SUCC(pthread_create(&thread_id, NULL, unshare_files_thread,
				 (void *)(intptr_t)test_fd));
	TEST_SUCC(pthread_join(thread_id, NULL));

	TEST_RES(fstat(test_fd, &stat2), stat1.st_ino == stat2.st_ino);
	TEST_SUCC(close(test_fd));
	TEST_SUCC(unlink(TEST_FILENAME));
}
END_TEST()

#define CWD_BUF_SIZE 1024
#define THREAD_CWD "/tmp"

void *unshare_fs_thread(void *arg)
{
	CHECK(unshare(CLONE_FS));

	CHECK(chdir(THREAD_CWD));
	CHECK(getcwd((char *)arg, CWD_BUF_SIZE));

	return NULL;
}

FN_TEST(unshare_fs)
{
	char cwd_buf1[CWD_BUF_SIZE], cwd_buf2[CWD_BUF_SIZE];
	pthread_t thread_id;

	TEST_RES(getcwd(cwd_buf1, CWD_BUF_SIZE),
		 strcmp(cwd_buf1, THREAD_CWD) != 0);

	TEST_SUCC(pthread_create(&thread_id, NULL, unshare_fs_thread,
				 (void *)cwd_buf2));

	TEST_RES(pthread_join(thread_id, NULL),
		 strcmp(cwd_buf2, THREAD_CWD) == 0);

	TEST_RES(getcwd(cwd_buf2, CWD_BUF_SIZE),
		 strcmp(cwd_buf1, cwd_buf2) == 0);
}
END_TEST()
