// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <string.h>
#include <errno.h>
#include "../network/test.h"

#define FILE_SIZE 4096

const char *msg1 = "jguk&auyg#eufg\n";
const char *msg2 = "kug*skhikf%hsd\n";
const char *msg_combined = "jguk&auyg#eufg\nkug*skhikf%hsd\n";

FN_TEST(memfd_create)
{
	int fd = TEST_SUCC(
		memfd_create("test_memfd", MFD_CLOEXEC | MFD_ALLOW_SEALING));

	TEST_RES(fcntl(fd, F_GETFD), (_ret & FD_CLOEXEC) == FD_CLOEXEC);
	TEST_SUCC(ftruncate(fd, FILE_SIZE));

	pid_t pid = TEST_SUCC(fork());

	if (pid == 0) {
		// Child process.

		// Write to the `memfd` file with the syscall `write`.
		write(fd, msg1, strlen(msg1));

		// Write to the `memfd` file with the syscall `mmap`.
		void *addr = mmap(NULL, FILE_SIZE, PROT_READ | PROT_WRITE,
				  MAP_SHARED, fd, 0);
		if (addr == MAP_FAILED) {
			perror("mmap failed");
			exit(1);
		}
		memcpy((char *)addr + strlen(msg1), msg2, strlen(msg2));
		munmap(addr, FILE_SIZE);

		close(fd);
		exit(0);
	} else {
		// Parent process.

		// Wait for the child to write contents.
		wait(NULL);

		// Read from the `memfd` file with the syscall `read`.
		lseek(fd, 0, SEEK_SET);
		char buffer[64] = { 0 };
		read(fd, buffer, sizeof(buffer) - 1);
		TEST_RES(strcmp(msg_combined, buffer), _ret == 0);

		// Read from the `memfd` file with the syscall `mmap`.
		void *addr = mmap(NULL, FILE_SIZE, PROT_READ | PROT_WRITE,
				  MAP_SHARED, fd, 0);
		if (addr == MAP_FAILED) {
			perror("mmap failed");
			exit(1);
		}
		TEST_RES(strcmp(msg_combined, (char *)addr), _ret == 0);
		munmap(addr, FILE_SIZE);

		close(fd);
	}
}
END_TEST()
