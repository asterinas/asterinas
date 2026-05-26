// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define CHILD_PATH "/test/process/execve/execve_comm_child"
/* `/proc/self/comm` truncates the executable name to 15 visible bytes. */
#define CHILD_COMM "execve_comm_chi"
#define CHILD_COMM_ENV "EXPECTED_COMM=" CHILD_COMM
#define LINK_PATH "/tmp/exec_link"
#define LINK_NAME "exec_link"
#define LINK_NAME_ENV "EXPECTED_COMM=" LINK_NAME

FN_TEST(execve_symlink)
{
	char *const argv[] = { "custom-argv0", NULL };
	char *const envp[] = { LINK_NAME_ENV, NULL };
	int status;
	pid_t pid;

	TEST_SUCC(symlink(CHILD_PATH, LINK_PATH));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		execve(LINK_PATH, argv, envp);
		_exit(127);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	TEST_SUCC(unlink(LINK_PATH));
}
END_TEST()

FN_TEST(execveat_empty_path)
{
	char *const argv[] = { "custom-argv0", NULL };
	char *const envp[] = { CHILD_COMM_ENV, NULL };
	int child_fd;
	int status;
	pid_t pid;

	child_fd = TEST_SUCC(open(CHILD_PATH, O_RDONLY));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		execveat(child_fd, "", argv, envp, AT_EMPTY_PATH);
		_exit(127);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	TEST_SUCC(close(child_fd));
}
END_TEST()
