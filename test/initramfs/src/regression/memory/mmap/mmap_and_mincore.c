// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <sys/mman.h>
#include <unistd.h>

#include "../../common/test.h"

#define PAGE_SIZE 4096
#define FILE_NAME "/tmp/mincore_test.txt"

FN_TEST(anonymous_residency)
{
	unsigned char *anon_addr =
		TEST_SUCC(mmap(NULL, PAGE_SIZE * 2, PROT_READ | PROT_WRITE,
			       MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));

	unsigned char vec[2] = { 0xff, 0xff };

	TEST_RES(mincore(anon_addr, PAGE_SIZE * 2, vec),
		 _ret == 0 && (vec[0] & 1) == 0 && (vec[1] & 1) == 0);

	anon_addr[0] = 'x';
	vec[0] = vec[1] = 0xff;
	TEST_RES(mincore(anon_addr, PAGE_SIZE * 2, vec),
		 _ret == 0 && (vec[0] & 1) == 1 && (vec[1] & 1) == 0);

	anon_addr[PAGE_SIZE] = 'y';
	vec[0] = vec[1] = 0xff;
	TEST_RES(mincore(anon_addr, PAGE_SIZE * 2, vec),
		 _ret == 0 && (vec[0] & 1) == 1 && (vec[1] & 1) == 1);

	TEST_SUCC(munmap(anon_addr, PAGE_SIZE * 2));
}
END_TEST()

FN_TEST(filebacked_page_cache_residency)
{
	unsigned char *file_addr;
	int file_fd =
		TEST_SUCC(open(FILE_NAME, O_RDWR | O_CREAT | O_TRUNC, 0666));
	TEST_SUCC(ftruncate(file_fd, PAGE_SIZE * 2));
	TEST_SUCC(write(file_fd, "a", 1));
	file_addr = TEST_SUCC(
		mmap(NULL, PAGE_SIZE * 2, PROT_READ, MAP_SHARED, file_fd, 0));

	unsigned char vec[2] = { 0xff, 0xff };

	// For file-backed mappings, a page is considered resident
	// if its page cache is already committed,
	// even when the page is not mapped in the page table yet.
	TEST_RES(mincore(file_addr, PAGE_SIZE * 2, vec),
		 _ret == 0 && (vec[0] & 1) == 1 && (vec[1] & 1) == 0);

	TEST_SUCC(munmap(file_addr, PAGE_SIZE * 2));
	TEST_SUCC(close(file_fd));
	TEST_SUCC(unlink(FILE_NAME));
}
END_TEST()

FN_TEST(large_range_residency)
{
	static unsigned char vec[10000];
	size_t pages = sizeof(vec);
	size_t size = pages * PAGE_SIZE;

	unsigned char *addr =
		TEST_SUCC(mmap(NULL, size, PROT_READ | PROT_WRITE,
			       MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));

	// Make only the first and last pages resident.
	addr[0] = 'a';
	addr[(pages - 1) * PAGE_SIZE] = 'b';

	for (size_t i = 0; i < pages; i++)
		vec[i] = 0xff;

	TEST_RES(mincore(addr, size, vec),
		 _ret == 0 && (vec[0] & 1) == 1 && (vec[pages - 1] & 1) == 1);

	// Every other page must be reported non-resident.
	int middle_all_zero = 1;
	for (size_t i = 1; i < pages - 1; i++)
		middle_all_zero = middle_all_zero && ((vec[i] & 1) == 0);
	TEST_RES(middle_all_zero, _ret == 1);

	TEST_SUCC(munmap(addr, size));
}
END_TEST()
