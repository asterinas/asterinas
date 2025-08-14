// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../test.h"

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <string.h>
#include <errno.h>

FN_TEST(pid_mem)
{
	const char *old_text = "Hello#1, /proc/pid/mem!";
	const char *new_text = "Hello#2, /proc/pid/mem!";

	void *addr = CHECK_WITH(mmap(NULL, 4096, PROT_READ | PROT_WRITE,
				     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
				_ret != MAP_FAILED);

	strncpy(addr, old_text, strlen(old_text) + 1);

	char mem_path[64];
	pid_t pid = getpid();
	snprintf(mem_path, sizeof(mem_path), "/proc/%d/mem", pid);
	int fd = TEST_SUCC(open(mem_path, O_RDWR));

	char buf[256] = { 0 };

	TEST_SUCC(lseek(fd, (off_t)addr, SEEK_SET));
	TEST_SUCC(read(fd, buf, sizeof(buf) - 1));
	TEST_RES(strcmp(buf, old_text), _ret == 0);

	TEST_SUCC(lseek(fd, (off_t)addr, SEEK_SET));
	TEST_SUCC(write(fd, new_text, strlen(new_text)));
	TEST_RES(strcmp(addr, new_text), _ret == 0);

	close(fd);
	munmap(addr, 4096);
}
END_TEST()
