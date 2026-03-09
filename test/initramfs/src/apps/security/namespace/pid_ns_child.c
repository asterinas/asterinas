// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

static ssize_t read_link_value(const char *path, char *buf, size_t buf_size)
{
	ssize_t len = readlink(path, buf, buf_size - 1);

	if (len >= 0) {
		buf[len] = '\0';
	}

	return len;
}

static ssize_t read_pid_ns_link(pid_t pid, char *buf, size_t buf_size)
{
	char path[PATH_MAX];

	snprintf(path, sizeof(path), "/proc/%d/ns/pid", pid);
	return read_link_value(path, buf, buf_size);
}

int main(void)
{
	int release_pipe[2];
	int status = 0;
	char active_link[PATH_MAX];
	char child_link[PATH_MAX];

	if (read_link_value("/proc/self/ns/pid", active_link,
			    sizeof(active_link)) < 0) {
		perror("readlink(/proc/self/ns/pid)");
		return EXIT_FAILURE;
	}

	if (pipe(release_pipe) < 0) {
		perror("pipe");
		return EXIT_FAILURE;
	}

	pid_t child = fork();
	if (child < 0) {
		perror("fork");
		return EXIT_FAILURE;
	}

	if (child == 0) {
		char release = '\0';

		close(release_pipe[1]);

		if (getpid() != 1) {
			fprintf(stderr,
				"child did not become pid 1 after execve\n");
			_exit(EXIT_FAILURE);
		}

		if (read(release_pipe[0], &release, 1) != 1) {
			perror("read");
			_exit(EXIT_FAILURE);
		}

		close(release_pipe[0]);
		_exit(EXIT_SUCCESS);
	}

	close(release_pipe[0]);

	if (read_pid_ns_link(child, child_link, sizeof(child_link)) < 0) {
		perror("readlink(/proc/<child>/ns/pid)");
		close(release_pipe[1]);
		return EXIT_FAILURE;
	}

	if (strcmp(active_link, child_link) == 0) {
		fprintf(stderr,
			"child unexpectedly stayed in the active pid namespace: %s == %s\n",
			active_link, child_link);
		close(release_pipe[1]);
		return EXIT_FAILURE;
	}

	if (write(release_pipe[1], "X", 1) != 1) {
		perror("write");
		close(release_pipe[1]);
		return EXIT_FAILURE;
	}

	close(release_pipe[1]);

	if (waitpid(child, &status, 0) != child) {
		perror("waitpid");
		return EXIT_FAILURE;
	}

	return WIFEXITED(status) && WEXITSTATUS(status) == 0 ? EXIT_SUCCESS :
							       EXIT_FAILURE;
}
