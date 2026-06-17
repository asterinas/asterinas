// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <sys/inotify.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define CHILD_NAME "execve_comm_child"
/* `/proc/self/comm` truncates the executable name to 15 visible bytes. */
#define CHILD_COMM "execve_comm_chi"
#define CHILD_COMM_ENV "EXPECTED_COMM=" CHILD_COMM
#define LINK_PATH "/tmp/exec_link"
#define LINK_NAME "exec_link"
#define LINK_NAME_ENV "EXPECTED_COMM=" LINK_NAME

static void get_child_path(char *path, size_t size)
{
	char self_path[256];
	ssize_t len;
	char *last_slash;

	len = CHECK(
		readlink("/proc/self/exe", self_path, sizeof(self_path) - 1));
	self_path[len] = '\0';

	last_slash = CHECK_WITH(strrchr(self_path, '/'), _ret != NULL);
	*last_slash = '\0';
	CHECK_WITH(snprintf(path, size, "%s/%s", self_path, CHILD_NAME),
		   _ret < (int)size);
}

static void split_dirname(char *path, char **dir, char **name)
{
	char *last_slash = CHECK_WITH(strrchr(path, '/'), _ret != NULL);

	*last_slash = '\0';
	*dir = path;
	*name = last_slash + 1;
}

static int wait_for_inotify_events(int inotify_fd, const char *name,
				   unsigned int expected)
{
	char buf[4096]
		__attribute__((aligned(__alignof__(struct inotify_event))));
	unsigned int seen = 0;

	while ((seen & expected) != expected) {
		struct pollfd poll_fd = {
			.fd = inotify_fd,
			.events = POLLIN,
		};

		if (poll(&poll_fd, 1, 5000) <= 0) {
			return -1;
		}

		ssize_t len = read(inotify_fd, buf, sizeof(buf));
		if (len < 0) {
			return -1;
		}

		for (char *ptr = buf; ptr < buf + len;) {
			struct inotify_event *event =
				(struct inotify_event *)ptr;

			if (event->len != 0 && strcmp(event->name, name) == 0) {
				seen |= event->mask;
			}

			ptr += sizeof(*event) + event->len;
		}
	}

	return 0;
}

FN_TEST(execve_symlink)
{
	char *const argv[] = { "custom-argv0", NULL };
	char *const envp[] = { LINK_NAME_ENV, NULL };
	char child_path[256];
	int status;
	pid_t pid;

	get_child_path(child_path, sizeof(child_path));
	TEST_SUCC(symlink(child_path, LINK_PATH));

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
	char child_comm_env[64];
	char *const envp[] = { child_comm_env, NULL };
	int child_fd;
	int status;
	pid_t pid;

	char child_path[256];
	get_child_path(child_path, sizeof(child_path));
	child_fd = TEST_SUCC(open(child_path, O_RDONLY));
#ifdef __asterinas__
	CHECK_WITH(snprintf(child_comm_env, sizeof(child_comm_env), "%s",
			    CHILD_COMM_ENV),
		   _ret < (int)sizeof(child_comm_env));
#else
	CHECK_WITH(snprintf(child_comm_env, sizeof(child_comm_env),
			    "EXPECTED_COMM=%d", child_fd),
		   _ret < (int)sizeof(child_comm_env));
#endif

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

FN_TEST(execve_opens_executable_file)
{
	char *const argv[] = { "custom-argv0", NULL };
	char *const envp[] = { CHILD_COMM_ENV, NULL };
	char child_path[256];
	char watch_path[256];
	char *child_dir;
	char *child_name;
	int inotify_fd;
	int status;
	int wd;
	pid_t pid;

	get_child_path(child_path, sizeof(child_path));
	CHECK_WITH(snprintf(watch_path, sizeof(watch_path), "%s", child_path),
		   _ret < (int)sizeof(watch_path));
	split_dirname(watch_path, &child_dir, &child_name);

	inotify_fd = TEST_SUCC(inotify_init1(0));
	wd = TEST_SUCC(inotify_add_watch(
		inotify_fd, child_dir, IN_OPEN | IN_ACCESS | IN_CLOSE_NOWRITE));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		execve(child_path, argv, envp);
		_exit(127);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_RES(wait_for_inotify_events(inotify_fd, child_name,
					 IN_OPEN | IN_ACCESS |
						 IN_CLOSE_NOWRITE),
		 _ret == 0);

	TEST_SUCC(inotify_rm_watch(inotify_fd, wd));
	TEST_SUCC(close(inotify_fd));
}
END_TEST()
