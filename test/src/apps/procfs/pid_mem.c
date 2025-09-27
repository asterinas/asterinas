// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <stdint.h>
#include <string.h>

#include "../test.h"

#define PAGE_SIZE 4096
#define ORIG_STR "ORIGINAL"
#define NEW_STR "MODIFIED"
const char *filename = "testfile";

FN_TEST(proc_mem_private)
{
	int fd = TEST_SUCC(open(filename, O_RDWR | O_CREAT | O_TRUNC, 0600));
	TEST_SUCC(ftruncate(fd, PAGE_SIZE));
	TEST_SUCC(write(fd, ORIG_STR, strlen(ORIG_STR) + 1));
	TEST_SUCC(close(fd));

	int pipe_c2p[2], pipe_p2c[2];
	TEST_SUCC(pipe(pipe_c2p));
	TEST_SUCC(pipe(pipe_p2c));

	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		// ===== Child =====
		TEST_SUCC(close(pipe_c2p[0]));
		TEST_SUCC(close(pipe_p2c[1]));

		int fd = TEST_SUCC(open(filename, O_RDONLY));
		void *addr =
			CHECK_WITH(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
					MAP_PRIVATE, fd, 0),
				   _ret != MAP_FAILED);
		TEST_SUCC(write(pipe_c2p[1], &addr, sizeof(addr)));

		// Wait for parent to read and write
		char ack;
		TEST_SUCC(read(pipe_p2c[0], &ack, 1));

		// Check if the memory was modified by the parent
		TEST_RES(memcmp(addr, NEW_STR, strlen(NEW_STR)), _ret == 0);

		TEST_SUCC(munmap(addr, PAGE_SIZE));
		TEST_SUCC(close(fd));
		TEST_SUCC(close(pipe_c2p[1]));
		TEST_SUCC(close(pipe_p2c[0]));
		exit(EXIT_SUCCESS);
	} else {
		// ===== Parent =====
		TEST_SUCC(close(pipe_c2p[1]));
		TEST_SUCC(close(pipe_p2c[0]));

		void *child_vaddr;
		TEST_SUCC(read(pipe_c2p[0], &child_vaddr, sizeof(child_vaddr)));

		char mempath[256];
		snprintf(mempath, sizeof(mempath), "/proc/%d/mem", (int)child);
		int proc_mem_fd = TEST_SUCC(open(mempath, O_RDWR));

		// Read from child's memory via /proc/pid/mem
		TEST_SUCC(lseek(proc_mem_fd, (off_t)child_vaddr, SEEK_SET));
		char readbuf[64] = { 0 };
		TEST_SUCC(read(proc_mem_fd, readbuf, sizeof(readbuf)));
		TEST_RES(strncmp(readbuf, ORIG_STR, strlen(ORIG_STR)),
			 _ret == 0);

		// Write to child's memory via /proc/pid/mem
		TEST_SUCC(lseek(proc_mem_fd, (off_t)child_vaddr, SEEK_SET));
		TEST_SUCC(write(proc_mem_fd, NEW_STR, strlen(NEW_STR) + 1));
		TEST_SUCC(close(proc_mem_fd));

		TEST_SUCC(write(pipe_p2c[1], "X", 1));

		int status;
		TEST_RES(wait4(child, &status, 0, NULL),
			 _ret == child && WIFEXITED(status) &&
				 WEXITSTATUS(status) == EXIT_SUCCESS);

		TEST_SUCC(close(pipe_c2p[0]));
		TEST_SUCC(close(pipe_p2c[1]));
		TEST_SUCC(unlink(filename));
	}
}
END_TEST()
