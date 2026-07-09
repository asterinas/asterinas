// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#define AT_NULL 0
#define AT_SECURE 23
#define BUFFER_SIZE 4096

static int read_text_file(const char *path, char *buffer, size_t buffer_size)
{
	int fd;
	ssize_t len;

	if (buffer_size == 0) {
		return -1;
	}

	fd = open(path, O_RDONLY);
	if (fd < 0) {
		return -1;
	}

	len = read(fd, buffer, buffer_size - 1);
	if (len < 0) {
		close(fd);
		return -1;
	}
	buffer[len] = '\0';

	return close(fd);
}

static int write_text_file(const char *path, const char *text)
{
	int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
	size_t len = strlen(text);
	size_t written = 0;

	if (fd < 0) {
		return -1;
	}

	while (written < len) {
		ssize_t count = write(fd, text + written, len - written);

		if (count < 0) {
			close(fd);
			return -1;
		}
		written += (size_t)count;
	}

	return close(fd);
}

static int read_at_secure(unsigned long *secure)
{
	unsigned long entry[2];
	int fd = open("/proc/self/auxv", O_RDONLY);

	if (fd < 0) {
		return -1;
	}

	while (read(fd, entry, sizeof(entry)) == sizeof(entry)) {
		if (entry[0] == AT_SECURE) {
			*secure = entry[1] ? 1 : 0;
			return close(fd);
		}
		if (entry[0] == AT_NULL) {
			break;
		}
	}

	close(fd);
	return -1;
}

static int write_at_secure(const char *path)
{
	unsigned long secure;
	char text[4];

	if (read_at_secure(&secure) < 0) {
		return -1;
	}
	snprintf(text, sizeof(text), "%lu\n", secure);

	return write_text_file(path, text);
}

int main(int argc, char *argv[])
{
	char current[BUFFER_SIZE];

	if (argc != 2 && argc != 3) {
		return 1;
	}
	if (read_text_file("/proc/self/attr/current", current,
			   sizeof(current)) < 0) {
		return 2;
	}
	if (write_text_file(argv[1], current) < 0) {
		return 3;
	}
	if (argc == 3 && write_at_secure(argv[2]) < 0) {
		return 4;
	}

	return 0;
}
