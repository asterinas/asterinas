// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define EXECUTABLE_PATH "/test/process/execve/hello"

static int check_execveat_result(int fd, const char *filename, int flags,
				 int expected_errno)
{
	char *const argv[] = { "execveat-child", NULL };
	char *const envp[] = { NULL };
	pid_t pid = fork();
	if (pid < 0)
		return -1;

	if (pid == 0) {
		execveat(fd, filename, argv, envp, flags);

		int exec_errno = errno;
		if (exec_errno != expected_errno) {
			fprintf(stderr, "execveat returned %s, expected %s\n",
				strerror(exec_errno), strerror(expected_errno));
		}
		_exit(exec_errno == expected_errno ? EXIT_SUCCESS :
						     EXIT_FAILURE);
	}

	int status;
	if (waitpid(pid, &status, 0) != pid)
		return -1;

	return WIFEXITED(status) && WEXITSTATUS(status) == EXIT_SUCCESS ? 0 :
									  -1;
}

FN_TEST(execveat_cloexec)
{
	char test_dir[] = "/tmp/execveat-shebang-cloexec-XXXXXX";
	char script_path[sizeof(test_dir) + sizeof("/script")];
	static const char script[] = "#!/bin/sh\nexit 42\n";

	int executable_fd =
		TEST_SUCC(open(EXECUTABLE_PATH, O_RDONLY | O_CLOEXEC));
	TEST_RES(check_execveat_result(executable_fd, "", AT_EMPTY_PATH, 0),
		 _ret == 0);
	TEST_SUCC(close(executable_fd));

	TEST_RES(mkdtemp(test_dir), _ret != NULL);
	TEST_RES(snprintf(script_path, sizeof(script_path), "%s/script",
			  test_dir),
		 _ret > 0 && (size_t)_ret < sizeof(script_path));

	int script_create_fd = TEST_SUCC(
		open(script_path, O_WRONLY | O_CREAT | O_TRUNC, 0700));
	TEST_RES(write(script_create_fd, script, sizeof(script) - 1),
		 _ret == (ssize_t)(sizeof(script) - 1));
	TEST_SUCC(close(script_create_fd));
	TEST_SUCC(chmod(script_path, 0700));

	int script_fd = TEST_SUCC(open(script_path, O_RDONLY | O_CLOEXEC));
	TEST_RES(check_execveat_result(script_fd, "", AT_EMPTY_PATH, ENOENT),
		 _ret == 0);
	TEST_SUCC(close(script_fd));

	int dirfd =
		TEST_SUCC(open(test_dir, O_RDONLY | O_DIRECTORY | O_CLOEXEC));
	TEST_RES(check_execveat_result(dirfd, "script", 0, ENOENT), _ret == 0);
	TEST_SUCC(close(dirfd));

	TEST_SUCC(unlink(script_path));
	TEST_SUCC(rmdir(test_dir));
}
END_TEST()
