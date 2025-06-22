// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>
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
	char *addr1 = x_mmap(NULL, PAGE_SIZE * 2, PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	strcpy(addr1, content);

	// Unmap a target region to ensure we know it's free
	char *addr2 = addr1 + PAGE_SIZE;
	TEST_SUCC(munmap(addr2, PAGE_SIZE)); // free it for mremap

	// Remap from the first address to the second address
	if (mremap(addr1, PAGE_SIZE, PAGE_SIZE, MREMAP_MAYMOVE | MREMAP_FIXED,
		   addr2) != addr2) {
		perror("mremap");
		exit(EXIT_FAILURE);
	}
	TEST_RES(strcmp(addr2, content), _ret == 0);

	// Remap from the second address to the first address
	if (mremap(addr2, PAGE_SIZE, PAGE_SIZE, MREMAP_MAYMOVE | MREMAP_FIXED,
		   addr1) != addr1) {
		perror("mremap");
		exit(EXIT_FAILURE);
	}
	TEST_RES(strcmp(addr1, content), _ret == 0);

	TEST_SUCC(munmap(addr1, PAGE_SIZE));
}
END_TEST()
