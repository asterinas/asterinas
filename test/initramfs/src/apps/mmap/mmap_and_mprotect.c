// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <sys/fcntl.h>
#include <sys/mman.h>
#include <unistd.h>
#include <string.h>

#include "../test.h"

#define PAGE_SIZE 4096
const char *filename = "testfile";

FN_TEST(mprotect_shared_writable_mapping_on_read_only_file)
{
	int fd = TEST_SUCC(open(filename, O_RDWR | O_CREAT | O_TRUNC, 0600));
	TEST_SUCC(ftruncate(fd, PAGE_SIZE));
	TEST_SUCC(write(fd, "AAAA", 5));

	TEST_SUCC(close(fd));
	fd = TEST_SUCC(open(filename, O_RDONLY));

	char *addr =
		TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_SHARED, fd, 0));
	TEST_ERRNO(mprotect(addr, PAGE_SIZE, PROT_READ | PROT_WRITE), EACCES);

	TEST_SUCC(munmap(addr, PAGE_SIZE));
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(filename));
}
END_TEST()

FN_TEST(mprotect_private_writable_mapping_copy_on_write)
{
	int fd = TEST_SUCC(open(filename, O_RDWR | O_CREAT | O_TRUNC, 0600));
	TEST_SUCC(ftruncate(fd, PAGE_SIZE));
	TEST_SUCC(write(fd, "AAAA", 5));

	char *addr1 =
		TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_PRIVATE, fd, 0));
	char *addr2 =
		TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_PRIVATE, fd, 0));
	TEST_RES(strcmp(addr1, "AAAA"), _ret == 0);
	TEST_RES(strcmp(addr2, "AAAA"), _ret == 0);
	TEST_SUCC(mprotect(addr1, PAGE_SIZE, PROT_READ | PROT_WRITE));
	memcpy(addr1, "BBBB", 5);
	TEST_RES(strcmp(addr1, "BBBB"), _ret == 0);
	TEST_RES(strcmp(addr2, "AAAA"), _ret == 0);

	TEST_SUCC(munmap(addr1, PAGE_SIZE));
	TEST_SUCC(munmap(addr2, PAGE_SIZE));
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(filename));
}
END_TEST()

FN_TEST(mprotect_invalid_permission)
{
#define PROT_INVALID 0x20

	void *addr = TEST_SUCC(mmap(NULL, PAGE_SIZE,
				    PROT_READ | PROT_WRITE | PROT_INVALID,
				    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));

	TEST_SUCC(mprotect(addr, PAGE_SIZE, PROT_READ | PROT_EXEC));
	TEST_ERRNO(mprotect(addr, PAGE_SIZE, PROT_READ | PROT_INVALID), EINVAL);

	TEST_SUCC(munmap(addr, PAGE_SIZE));
}
END_TEST()
