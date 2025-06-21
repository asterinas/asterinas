// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include "../network/test.h"

#define PAGE_SIZE 4096

const char *content = "kjfkljk*wigo&h";

void *x_mmap(void *addr, size_t length, int prot, int flags, int fd,
	     off_t offset)
{
	void *result = mmap(addr, length, prot, flags, fd, offset);
	if (result == MAP_FAILED) {
		perror("mmap");
		exit(EXIT_FAILURE);
	}
	return result;
}

FN_TEST(mmap_and_mremap)
{
	char *addr = x_mmap(NULL, 3 * PAGE_SIZE, PROT_READ | PROT_WRITE,
			    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	TEST_SUCC(munmap(addr, 3 * PAGE_SIZE));

	addr = x_mmap(addr, PAGE_SIZE, PROT_READ | PROT_WRITE,
		      MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
	strcpy(addr, content);

	char *addr2 = x_mmap(addr + 2 * PAGE_SIZE, PAGE_SIZE,
			     PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);

	char *new_addr = mremap(addr, PAGE_SIZE, 3 * PAGE_SIZE, MREMAP_MAYMOVE);
	if (new_addr == MAP_FAILED) {
		perror("mremap");
		exit(EXIT_FAILURE);
	}

	// The following operation (if uncommented) would cause a segmentation fault.
	// strcpy(addr, "Writing to old address");

	TEST_RES(strcmp(new_addr, content), _ret == 0);
	strcpy(new_addr + PAGE_SIZE, "Writing to page 2 (new)");
	strcpy(new_addr + 2 * PAGE_SIZE, "Writing to page 3 (new)");

	TEST_SUCC(munmap(new_addr, 3 * PAGE_SIZE));
	TEST_SUCC(munmap(addr2, PAGE_SIZE));
}
END_TEST()

FN_TEST(mmap_and_mremap_fixed)
{
	char *addr = x_mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
			    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	strcpy(addr, content);

	// Map and unmap a target region to ensure we know it's free
	char *fixed_addr = x_mmap(NULL, PAGE_SIZE, PROT_NONE,
				  MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	TEST_SUCC(munmap(fixed_addr, PAGE_SIZE)); // free it for mremap

	char *new_addr = mremap(addr, PAGE_SIZE, PAGE_SIZE,
				MREMAP_MAYMOVE | MREMAP_FIXED, fixed_addr);
	if (new_addr != fixed_addr) {
		perror("mremap");
		exit(EXIT_FAILURE);
	}

	TEST_RES(strcmp(new_addr, content), _ret == 0);
	TEST_SUCC(munmap(new_addr, PAGE_SIZE));
}
END_TEST()

FN_TEST(mmap_and_mremap_auto_merge_anon)
{
	char *addr = x_mmap(NULL, 6 * PAGE_SIZE, PROT_READ | PROT_WRITE,
			    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	TEST_SUCC(munmap(addr, 6 * PAGE_SIZE));

	x_mmap(addr, PAGE_SIZE, PROT_READ | PROT_WRITE,
	       MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
	strcpy(addr, content);
	x_mmap(addr + 2 * PAGE_SIZE, PAGE_SIZE, PROT_READ | PROT_WRITE,
	       MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
	x_mmap(addr + PAGE_SIZE, PAGE_SIZE, PROT_READ | PROT_WRITE,
	       MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);

	char *new_addr = mremap(addr, 3 * PAGE_SIZE, 3 * PAGE_SIZE,
				MREMAP_MAYMOVE | MREMAP_FIXED,
				addr + 3 * PAGE_SIZE);
	if (new_addr == MAP_FAILED) {
		perror("mremap");
		exit(EXIT_FAILURE);
	}

	TEST_RES(strcmp(new_addr, content), _ret == 0);
	TEST_SUCC(munmap(new_addr, 3 * PAGE_SIZE));
}
END_TEST()

FN_TEST(mmap_and_mremap_auto_merge_file)
{
	const char *filename = "mremap_test_file";
	int fd = TEST_SUCC(open(filename, O_CREAT | O_RDWR, 0600));
	TEST_SUCC(ftruncate(fd, 6 * PAGE_SIZE));

	char *addr = x_mmap(NULL, 6 * PAGE_SIZE, PROT_READ | PROT_WRITE,
			    MAP_PRIVATE, fd, 0);
	TEST_SUCC(munmap(addr, 6 * PAGE_SIZE));

	x_mmap(addr, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_FIXED,
	       fd, 0);
	strcpy(addr, content);
	x_mmap(addr + 2 * PAGE_SIZE, PAGE_SIZE, PROT_READ | PROT_WRITE,
	       MAP_PRIVATE | MAP_FIXED, fd, 2 * PAGE_SIZE);
	x_mmap(addr + PAGE_SIZE, PAGE_SIZE, PROT_READ | PROT_WRITE,
	       MAP_PRIVATE | MAP_FIXED, fd, PAGE_SIZE);

	char *new_addr = mremap(addr, 3 * PAGE_SIZE, 3 * PAGE_SIZE,
				MREMAP_MAYMOVE | MREMAP_FIXED,
				addr + 3 * PAGE_SIZE);
	if (new_addr == MAP_FAILED) {
		perror("mremap");
		exit(EXIT_FAILURE);
	}

	TEST_RES(strcmp(new_addr, content), _ret == 0);
	TEST_SUCC(munmap(new_addr, 3 * PAGE_SIZE));
	close(fd);
	unlink(filename);
}
END_TEST()