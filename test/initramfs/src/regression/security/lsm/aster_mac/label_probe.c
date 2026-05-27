/* SPDX-License-Identifier: MPL-2.0 */

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#define CURRENT_LABEL_FILE "/proc/self/attr/current"

static int read_current_label(char *buffer, size_t buffer_len)
{
	if (buffer_len == 0) {
		errno = EINVAL;
		return -1;
	}

	int fd = open(CURRENT_LABEL_FILE, O_RDONLY);
	if (fd < 0) {
		return -1;
	}

	ssize_t read_len = read(fd, buffer, buffer_len - 1);
	int saved_errno = errno;
	close(fd);
	errno = saved_errno;
	if (read_len < 0) {
		return -1;
	}

	buffer[read_len] = '\0';
	buffer[strcspn(buffer, "\n")] = '\0';
	return 0;
}

static int expect_child_label(const char *expected)
{
	pid_t pid = fork();
	if (pid < 0) {
		return -1;
	}

	if (pid == 0) {
		char label[128];
		if (read_current_label(label, sizeof(label)) < 0) {
			_exit(10);
		}
		_exit(strcmp(label, expected) == 0 ? 0 : 11);
	}

	int status = 0;
	if (waitpid(pid, &status, 0) < 0) {
		return -1;
	}

	if (WIFEXITED(status) && WEXITSTATUS(status) == 0) {
		return 0;
	}

	errno = EIO;
	return -1;
}

int main(int argc, char *argv[])
{
	if (argc != 2) {
		fprintf(stderr, "usage: %s <expected-label>\n", argv[0]);
		return 2;
	}

	char label[128];
	if (read_current_label(label, sizeof(label)) < 0) {
		perror("read_current_label");
		return 3;
	}
	if (strcmp(label, argv[1]) != 0) {
		fprintf(stderr, "label mismatch: got %s, expected %s\n", label, argv[1]);
		return 4;
	}

	if (expect_child_label(argv[1]) < 0) {
		perror("expect_child_label");
		return 5;
	}

	return 0;
}
