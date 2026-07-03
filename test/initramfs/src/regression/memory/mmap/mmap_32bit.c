// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <sys/mman.h>
#include <unistd.h>

#include "../../common/test.h"

#define PAGE_SIZE 4096

FN_TEST(mmap_32bit)
{
	void *addr;

	addr = TEST_RES(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS | MAP_32BIT, -1, 0),
			(size_t)_ret < 0x80000000);
	TEST_SUCC(munmap(addr, PAGE_SIZE));

	addr = TEST_RES(mmap((void *)0x40000000, PAGE_SIZE,
			     PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS | MAP_32BIT, -1, 0),
			(size_t)_ret < 0x80000000);
	TEST_SUCC(munmap(addr, PAGE_SIZE));

	addr = TEST_RES(mmap((void *)0x100000000, PAGE_SIZE,
			     PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS | MAP_32BIT, -1, 0),
			(size_t)_ret < 0x80000000);
	TEST_SUCC(munmap(addr, PAGE_SIZE));

	addr = TEST_RES(mmap(NULL, PAGE_SIZE * 64, PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS | MAP_32BIT, -1, 0),
			(size_t)_ret < 0x80000000);
	TEST_SUCC(munmap(addr, PAGE_SIZE * 64));

	// `MAP_FIXED` takes precedence over `MAP_32BIT`; the mapping must be
	// placed at the fixed address (above 2 GiB) rather than constrained
	// below 2 GiB.
	addr = TEST_RES(
		mmap((void *)0x300000000, PAGE_SIZE, PROT_READ | PROT_WRITE,
		     MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED | MAP_32BIT, -1,
		     0),
		_ret == (void *)0x300000000);
	TEST_SUCC(munmap(addr, PAGE_SIZE));

	// Hint near the 2 GiB boundary with a size that would overflow past it;
	// the result must be constrained below 2 GiB.
	addr = TEST_RES(mmap((void *)0x7FFF0000, PAGE_SIZE * 256,
			     PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS | MAP_32BIT, -1, 0),
			(size_t)_ret < 0x80000000);
	TEST_SUCC(munmap(addr, PAGE_SIZE * 256));
}
END_TEST()
