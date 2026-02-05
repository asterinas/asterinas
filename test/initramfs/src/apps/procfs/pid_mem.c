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
#define FILE_NAME "testfile"

FN_TEST(proc_mem_remote)
{
	int fd = TEST_SUCC(open(FILE_NAME, O_RDWR | O_CREAT | O_TRUNC, 0600));
	TEST_SUCC(ftruncate(fd, PAGE_SIZE));
	TEST_SUCC(write(fd, ORIG_STR, strlen(ORIG_STR) + 1));
	TEST_SUCC(close(fd));

	int pipe_c2p[2], pipe_p2c[2];
	TEST_SUCC(pipe(pipe_c2p));
	TEST_SUCC(pipe(pipe_p2c));

	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		// ===== Child =====
		CHECK(close(pipe_c2p[0]));
		CHECK(close(pipe_p2c[1]));

		int fd = CHECK(open(FILE_NAME, O_RDONLY));
		// The parent should successfully read from and (force) write to this
		// memory region via `/proc/pid/mem`, although it isn't `PROT_WRITE`.
		void *addr = CHECK_WITH(mmap(NULL, PAGE_SIZE, PROT_READ,
					     MAP_PRIVATE, fd, 0),
					_ret != MAP_FAILED);
		CHECK(write(pipe_c2p[1], &addr, sizeof(addr)));

		// Wait for the parent to read and write.
		char ack;
		CHECK(read(pipe_p2c[0], &ack, 1));

		// Check that the memory was modified by the parent.
		CHECK_WITH(memcmp(addr, NEW_STR, strlen(NEW_STR)), _ret == 0);

		// Check that the file was not modified.
		char filebuf[64] = { 0 };
		CHECK(lseek(fd, 0, SEEK_SET));
		CHECK(read(fd, filebuf, sizeof(filebuf)));
		CHECK_WITH(strncmp(filebuf, ORIG_STR, strlen(ORIG_STR)),
			   _ret == 0);

		CHECK(munmap(addr, PAGE_SIZE));
		CHECK(close(fd));
		CHECK(close(pipe_c2p[1]));
		CHECK(close(pipe_p2c[0]));
		exit(EXIT_SUCCESS);
	}

	// ===== Parent =====
	TEST_SUCC(close(pipe_c2p[1]));
	TEST_SUCC(close(pipe_p2c[0]));

	void *child_vaddr;
	TEST_SUCC(read(pipe_c2p[0], &child_vaddr, sizeof(child_vaddr)));

	char mempath[256];
	snprintf(mempath, sizeof(mempath), "/proc/%d/mem", (int)child);
	int proc_mem_fd = TEST_SUCC(open(mempath, O_RDWR));

	// Read from the child's memory via /proc/pid/mem.
	// This will trigger a read page fault on the child process.
	TEST_SUCC(lseek(proc_mem_fd, (off_t)child_vaddr, SEEK_SET));
	char readbuf[64] = { 0 };
	TEST_SUCC(read(proc_mem_fd, readbuf, sizeof(readbuf)));
	TEST_RES(strncmp(readbuf, ORIG_STR, strlen(ORIG_STR)), _ret == 0);

	// Write to the child's memory via /proc/pid/mem.
	// This will trigger a write page fault and perform COW on the child process.
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
	TEST_SUCC(unlink(FILE_NAME));
}
END_TEST()

FN_TEST(proc_mem_local)
{
	int fd = TEST_SUCC(open(FILE_NAME, O_RDWR | O_CREAT | O_TRUNC, 0600));
	TEST_SUCC(ftruncate(fd, PAGE_SIZE));
	TEST_SUCC(write(fd, ORIG_STR, strlen(ORIG_STR) + 1));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(FILE_NAME, O_RDONLY));
	void *addr1 =
		TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_PRIVATE, fd, 0));
	void *addr2 = TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
				     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));

	int proc_mem_fd = TEST_SUCC(open("/proc/self/mem", O_RDWR));
	TEST_SUCC(lseek(proc_mem_fd, (off_t)addr1, SEEK_SET));
	// This `read` will first trigger a page fault on `addr1` to load the
	// corresponding file page into memory. Then it will trigger a write
	// page fault on `addr2` to copy the content of that file page.
	TEST_SUCC(read(proc_mem_fd, addr2, PAGE_SIZE));
	TEST_RES(strncmp(addr2, ORIG_STR, strlen(ORIG_STR)), _ret == 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(close(proc_mem_fd));
	TEST_SUCC(munmap(addr1, PAGE_SIZE));
	TEST_SUCC(munmap(addr2, PAGE_SIZE));
	TEST_SUCC(unlink(FILE_NAME));
}
END_TEST()
