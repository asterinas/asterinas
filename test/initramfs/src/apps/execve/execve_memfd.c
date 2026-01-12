// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <errno.h>
#include <assert.h>
#include "../test.h"

#define EXECUTABLE_PATH "/test/execve/hello"
#define MFD_NAME "test_memfd_execve"

FN_TEST(memfd_noexec_seal)
{
	int memfd = TEST_SUCC(
		memfd_create(MFD_NAME, MFD_CLOEXEC | MFD_NOEXEC_SEAL));

	TEST_RES(fcntl(memfd, F_GET_SEALS), _ret == F_SEAL_EXEC);

	struct stat st;
	TEST_RES(fstat(memfd, &st), (st.st_mode & 0777) == 0666);
	TEST_ERRNO(fchmod(memfd, 0777), EPERM);

	TEST_SUCC(close(memfd));
}
END_TEST()

FN_TEST(execveat_memfd)
{
	int hello_fd = TEST_SUCC(open(EXECUTABLE_PATH, O_RDONLY));
	int memfd = TEST_SUCC(
		memfd_create(MFD_NAME, MFD_CLOEXEC | MFD_ALLOW_SEALING));

	char buffer[65536];
	ssize_t bytes_read;
	while ((bytes_read =
			TEST_SUCC(read(hello_fd, buffer, sizeof(buffer))))) {
		TEST_RES(write(memfd, buffer, bytes_read), _ret == bytes_read);
	}
	TEST_SUCC(close(hello_fd));

	TEST_SUCC(fcntl(memfd, F_ADD_SEALS, F_SEAL_EXEC));
	TEST_RES(fcntl(memfd, F_GET_SEALS),
		 _ret == (F_SEAL_EXEC | F_SEAL_SHRINK | F_SEAL_GROW |
			  F_SEAL_WRITE | F_SEAL_FUTURE_WRITE));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		struct stat st;
		CHECK_WITH(fstat(memfd, &st),
			   _ret >= 0 && (st.st_mode & 0777) == 0777);
		CHECK_WITH(fchmod(memfd, 0666), _ret < 0 && errno == EPERM);

		char *const argv[] = { "memfd_hello", NULL };
		char *const envp[] = { "PATH=/bin:/usr/bin", NULL };
		CHECK(execveat(memfd, "", argv, envp, AT_EMPTY_PATH));

		exit(EXIT_FAILURE);
	}

	int status = 0;
	TEST_RES(wait4(pid, &status, 0, NULL),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()
