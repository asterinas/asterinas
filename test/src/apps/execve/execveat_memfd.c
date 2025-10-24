// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
#include <fcntl.h>
#include <errno.h>
#include <assert.h>
#include "../test.h"

#define EXECUTABLE_PATH "/test/execve/hello"
#define MFD_NAME "67890"

FN_TEST(execveat_memfd)
{
	int hello_fd = TEST_SUCC(open(EXECUTABLE_PATH, O_RDONLY));
	int memfd = TEST_SUCC(memfd_create(MFD_NAME, MFD_CLOEXEC));

	char buffer[65536];
	ssize_t bytes_read;
	while ((bytes_read =
			TEST_SUCC(read(hello_fd, buffer, sizeof(buffer))))) {
		TEST_RES(write(memfd, buffer, bytes_read), _ret == bytes_read);
	}
	TEST_SUCC(close(hello_fd));

	char *const argv[] = { "memfd_hello", NULL };
	char *const envp[] = { "PATH=/bin:/usr/bin", NULL };
	TEST_SUCC(execveat(memfd, "", argv, envp, AT_EMPTY_PATH));

	assert(0);
}
END_TEST()
