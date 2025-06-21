// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <sys/mman.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include "../test.h"

#define PAGE_SIZE 4096

FN_TEST(mremap)
{
	char *addr = TEST_SUCC(mmap(NULL, 3 * PAGE_SIZE, PROT_READ | PROT_WRITE,
				    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));
	TEST_SUCC(munmap(addr + 2 * PAGE_SIZE, PAGE_SIZE));

	// The old address is not page-aligned.
	TEST_ERRNO(mremap(addr + 1, PAGE_SIZE, PAGE_SIZE, 0), EINVAL);

	// The old size or the new size is not page-aligned.
	TEST_RES(mremap(addr, 1, PAGE_SIZE, 0), _ret == addr);
	TEST_RES(mremap(addr, PAGE_SIZE, 1, 0), _ret == addr);

	// The new address is not page-aligned.
	TEST_ERRNO(mremap(addr, PAGE_SIZE, PAGE_SIZE,
			  MREMAP_MAYMOVE | MREMAP_FIXED,
			  addr + 2 * PAGE_SIZE + 1),
		   EINVAL);

	// The flags or the combination of the flags is invalid.
	TEST_ERRNO(mremap(addr, PAGE_SIZE, PAGE_SIZE, MREMAP_FIXED,
			  addr + 2 * PAGE_SIZE),
		   EINVAL);
	TEST_ERRNO(mremap(addr, PAGE_SIZE, PAGE_SIZE, ~0, addr + 2 * PAGE_SIZE),
		   EINVAL);

	// Copying a private mapping should not be allowed. See the "BUGS" section at
	// <https://man7.org/linux/man-pages/man2/mremap.2.html>.
	TEST_ERRNO(mremap(addr, 0, PAGE_SIZE, 0), EINVAL);

	// There is no enough room to expand the mapping.
	// FIXME: Asterinas returns EACCESS here, which is not a correct error code.
	// TEST_ERRNO(mremap(addr, PAGE_SIZE, 2 * PAGE_SIZE, 0), ENOMEM);

	TEST_SUCC(munmap(addr, 2 * PAGE_SIZE));
}
END_TEST()

const char *content = "kjfkljk*wigo&h";

#define CHECK_MM(func) CHECK_WITH(func, _ret != MAP_FAILED)

FN_TEST(mmap_and_mremap)
{
	char *addr = CHECK_MM(mmap(NULL, 3 * PAGE_SIZE, PROT_READ | PROT_WRITE,
				   MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));
	TEST_SUCC(munmap(addr, 3 * PAGE_SIZE));

	addr = CHECK_MM(mmap(addr, PAGE_SIZE, PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0));
	strcpy(addr, content);

	char *addr2 = CHECK_MM(
		mmap(addr + 2 * PAGE_SIZE, PAGE_SIZE, PROT_READ | PROT_WRITE,
		     MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0));

	char *new_addr = CHECK_MM(
		mremap(addr, PAGE_SIZE, 3 * PAGE_SIZE, MREMAP_MAYMOVE));

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
	char *addr1 = CHECK_MM(mmap(NULL, PAGE_SIZE * 2, PROT_READ | PROT_WRITE,
				    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));
	strcpy(addr1, content);

	// Unmap a target region to ensure we know it's free
	char *addr2 = addr1 + PAGE_SIZE;
	TEST_SUCC(munmap(addr2, PAGE_SIZE)); // free it for mremap

	// Remap from the first address to the second address
	CHECK_WITH(mremap(addr1, PAGE_SIZE, PAGE_SIZE,
			  MREMAP_MAYMOVE | MREMAP_FIXED, addr2),
		   _ret == addr2);
	TEST_RES(strcmp(addr2, content), _ret == 0);

	// Remap from the second address to the first address
	CHECK_WITH(mremap(addr2, PAGE_SIZE, PAGE_SIZE,
			  MREMAP_MAYMOVE | MREMAP_FIXED, addr1),
		   _ret == addr1);
	TEST_RES(strcmp(addr1, content), _ret == 0);

	TEST_SUCC(munmap(addr1, PAGE_SIZE));
}
END_TEST()

FN_TEST(mmap_and_mremap_auto_merge_anon)
{
	char *addr = CHECK_MM(mmap(NULL, 6 * PAGE_SIZE, PROT_READ | PROT_WRITE,
				   MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));
	TEST_SUCC(munmap(addr, 6 * PAGE_SIZE));

	CHECK_MM(mmap(addr, PAGE_SIZE, PROT_READ | PROT_WRITE,
		      MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0));
	strcpy(addr, content);
	CHECK_MM(mmap(addr + 2 * PAGE_SIZE, PAGE_SIZE, PROT_READ | PROT_WRITE,
		      MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0));
	CHECK_MM(mmap(addr + PAGE_SIZE, PAGE_SIZE, PROT_READ | PROT_WRITE,
		      MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0));

	char *new_addr = CHECK_MM(mremap(addr, 3 * PAGE_SIZE, 3 * PAGE_SIZE,
					 MREMAP_MAYMOVE | MREMAP_FIXED,
					 addr + 3 * PAGE_SIZE));
	TEST_RES(strcmp(new_addr, content), _ret == 0);
	TEST_SUCC(munmap(new_addr, 3 * PAGE_SIZE));
}
END_TEST()

FN_TEST(mmap_and_mremap_auto_merge_file)
{
	const char *filename = "mremap_test_file";
	int fd = TEST_SUCC(open(filename, O_CREAT | O_RDWR, 0600));
	TEST_SUCC(ftruncate(fd, 6 * PAGE_SIZE));

	char *addr = CHECK_MM(mmap(NULL, 6 * PAGE_SIZE, PROT_READ | PROT_WRITE,
				   MAP_PRIVATE, fd, 0));
	TEST_SUCC(munmap(addr, 6 * PAGE_SIZE));

	CHECK_MM(mmap(addr, PAGE_SIZE, PROT_READ | PROT_WRITE,
		      MAP_PRIVATE | MAP_FIXED, fd, 0));
	strcpy(addr, content);
	CHECK_MM(mmap(addr + 2 * PAGE_SIZE, PAGE_SIZE, PROT_READ | PROT_WRITE,
		      MAP_PRIVATE | MAP_FIXED, fd, 2 * PAGE_SIZE));
	CHECK_MM(mmap(addr + PAGE_SIZE, PAGE_SIZE, PROT_READ | PROT_WRITE,
		      MAP_PRIVATE | MAP_FIXED, fd, PAGE_SIZE));

	char *new_addr = CHECK_MM(mremap(addr, 3 * PAGE_SIZE, 3 * PAGE_SIZE,
					 MREMAP_MAYMOVE | MREMAP_FIXED,
					 addr + 3 * PAGE_SIZE));
	TEST_RES(strcmp(new_addr, content), _ret == 0);
	TEST_SUCC(munmap(new_addr, 3 * PAGE_SIZE));

	close(fd);
	unlink(filename);
}
END_TEST()
