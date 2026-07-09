// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <errno.h>
#include <libgen.h>
#include <limits.h>
#include <linux/prctl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define NOBODY_UID 65534
#define CHILD_NAME "no_new_privs_child"

static char child_path[PATH_MAX];

static void exec_child(void)
{
	char *const argv[] = { CHILD_NAME, NULL };
	char *const envp[] = { NULL };

	CHECK(execve(child_path, argv, envp));
}

FN_SETUP(child_path)
{
	char self_path[PATH_MAX];
	ssize_t len = CHECK(
		readlink("/proc/self/exe", self_path, sizeof(self_path) - 1));
	self_path[len] = '\0';

	char *path_copy = CHECK_WITH(strdup(self_path), _ret != NULL);
	char *dir_name = CHECK_WITH(dirname(path_copy), _ret != NULL);

	CHECK_WITH(snprintf(child_path, sizeof(child_path), "%s/%s", dir_name,
			    CHILD_NAME),
		   _ret > 0 && (size_t)_ret < sizeof(child_path));
	free(path_copy);
}
END_SETUP()

FN_TEST(get_set_and_validate_args)
{
	pid_t pid;
	int status;

	TEST_RES(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 1, 0, 0, 0),
			   _ret == -1 && errno == EINVAL);
		CHECK_WITH(prctl(PR_SET_NO_NEW_PRIVS, 0, 0, 0, 0),
			   _ret == -1 && errno == EINVAL);
		CHECK_WITH(prctl(PR_SET_NO_NEW_PRIVS, 1, 1, 0, 0),
			   _ret == -1 && errno == EINVAL);
		CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
		CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 1);
		CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_RES(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);
}
END_TEST()

FN_TEST(fork_inherits_no_new_privs)
{
	pid_t pid;
	int status;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		pid_t grandchild;
		int grandchild_status;

		CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
		grandchild = CHECK(fork());
		if (grandchild == 0) {
			CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0),
				   _ret == 1);
			_exit(EXIT_SUCCESS);
		}

		CHECK_WITH(waitpid(grandchild, &grandchild_status, 0),
			   _ret == grandchild && WIFEXITED(grandchild_status) &&
				   WEXITSTATUS(grandchild_status) ==
					   EXIT_SUCCESS);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_TEST(exec_inherits_no_new_privs)
{
	pid_t pid;
	int status;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
		exec_child();
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_TEST(no_new_privs_suppresses_setuid_exec)
{
	pid_t pid;
	int status;
	struct stat child_stat;
	mode_t child_mode;

	SKIP_TEST_IF(geteuid() != 0);

	TEST_SUCC(stat(child_path, &child_stat));
	child_mode = child_stat.st_mode & 07777;
	TEST_SUCC(chmod(child_path, child_mode | S_ISUID));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setuid(NOBODY_UID));
		exec_child();
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
		CHECK(setuid(NOBODY_UID));
		exec_child();
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_SUCC(chmod(child_path, child_mode));
}
END_TEST()
