// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../test.h"

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <string.h>
#include <errno.h>

#define BUF_SIZE 256

FN_TEST(pid_mem)
{
	const char *old_text = "Hello#1, /proc/pid/mem!";
	const char *new_text = "Hello#2, /proc/pid/mem!";
	static volatile char addr[BUF_SIZE] = { 0 };

	pid_t pid = TEST_SUCC(fork());

	if (pid == 0) {
		// Child
		strncpy(addr, old_text, strlen(old_text) + 1);

		sleep(2); // Ensure parent has read and written
		CHECK_WITH(strcmp(addr, new_text), _ret == 0);

		exit(EXIT_SUCCESS);
	} else {
		// Parent
		char mem_path[64];
		snprintf(mem_path, sizeof(mem_path), "/proc/%d/mem", pid);
		int fd = TEST_SUCC(open(mem_path, O_RDWR));

		sleep(1); // Ensure child has written
		char buf[BUF_SIZE] = { 0 };

		TEST_SUCC(lseek(fd, (off_t)addr, SEEK_SET));
		TEST_SUCC(read(fd, buf, sizeof(buf) - 1));
		TEST_RES(strcmp(buf, old_text), _ret == 0);

		TEST_SUCC(lseek(fd, (off_t)addr, SEEK_SET));
		TEST_SUCC(write(fd, new_text, strlen(new_text)));
		close(fd);

		int status;
		TEST_RES(wait4(pid, &status, 0, NULL),
			 _ret == pid && WIFEXITED(status) &&
				 WEXITSTATUS(status) == EXIT_SUCCESS);
	}
}
END_TEST()
