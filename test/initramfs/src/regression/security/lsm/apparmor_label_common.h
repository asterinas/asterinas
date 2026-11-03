/* SPDX-License-Identifier: MPL-2.0 */

#ifndef APPARMOR_LABEL_COMMON_H
#define APPARMOR_LABEL_COMMON_H

#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <unistd.h>

#define AA_TEST_DIR "/tmp/aa-m2"
#define AA_HELPER "/test/security/lsm/apparmor_label_helper"

struct label_report {
	pid_t pid;
	char label[288];
};

static void transfer(int fd, void *buf, size_t size, int writing)
{
	char *bytes = buf;
	while (size) {
		struct pollfd wait = { .fd = fd,
				       .events = writing ? POLLOUT : POLLIN };
		int ready = poll(&wait, 1, 60000);
		if (ready < 0 && errno == EINTR)
			continue;
		assert(ready == 1);
		ssize_t count = writing ? write(fd, bytes, size) :
					  read(fd, bytes, size);
		if (count < 0 && errno == EINTR)
			continue;
		assert(count > 0);
		bytes += count;
		size -= count;
	}
}

static void read_label_fd(int fd, char *label, size_t size)
{
	/* One-byte preads exercise offsets, partial reads, and EOF on one open. */
	size_t offset = 0;
	for (; offset + 1 < size; offset++) {
		ssize_t count = pread(fd, label + offset, 1, offset);
		assert(count >= 0);
		if (!count)
			break;
	}
	assert(offset + 1 < size);
	label[offset] = '\0';
}

static void read_label(const char *path, char *label, size_t size)
{
	int fd = open(path, O_RDONLY);
	assert(fd >= 0);
	read_label_fd(fd, label, size);
	assert(close(fd) == 0);
}

#endif
