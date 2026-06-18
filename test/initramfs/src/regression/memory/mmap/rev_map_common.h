// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/wait.h>

#include "../../common/test.h"

#define PAGE_SIZE 4096

static int fd;
static char *mapped_buf;
static char read_buf[PAGE_SIZE] __attribute__((aligned(PAGE_SIZE)));
static char expected_buf[PAGE_SIZE] __attribute__((aligned(PAGE_SIZE)));

FN_SETUP(open)
{
	fd = CHECK(
		open(TEST_FILE, O_CREAT | O_TRUNC | O_RDWR | O_DIRECT, 0644));
	mapped_buf = CHECK_WITH(mmap(NULL, PAGE_SIZE * 2,
				     PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0),
				_ret != MAP_FAILED);

	CHECK(ftruncate(fd, PAGE_SIZE * 2));
}
END_SETUP()

// Mixing O_DIRECT access with cached access concurrently will corrupt the
// data. However, it should be fine if all accesses are sequential.

FN_TEST(memory_write_then_syscall_read)
{
	// Write by memory.
	strcpy(&mapped_buf[3], "hello");
	strcpy(&expected_buf[3], "hello");

	// Barrier.
	asm volatile("" : : : "memory");

	// Read by syscall.
	TEST_RES(pread(fd, read_buf, PAGE_SIZE, 0), _ret == PAGE_SIZE);
	TEST_RES(memcmp(read_buf, expected_buf, PAGE_SIZE), _ret == 0);

	// Read by memory.
	TEST_RES(memcmp(mapped_buf, expected_buf, PAGE_SIZE), _ret == 0);
}
END_TEST()

FN_TEST(syscall_write_then_memory_read)
{
	// Write by syscall.
	strcpy(&expected_buf[6], "world");
	TEST_RES(pwrite(fd, expected_buf, PAGE_SIZE, 0), _ret == PAGE_SIZE);

	// Barrier.
	asm volatile("" : : : "memory");

	// Read by memory.
	TEST_RES(memcmp(mapped_buf, expected_buf, PAGE_SIZE), _ret == 0);

	// Read by syscall.
	TEST_RES(pread(fd, read_buf, PAGE_SIZE, 0), _ret == PAGE_SIZE);
	TEST_RES(memcmp(read_buf, expected_buf, PAGE_SIZE), _ret == 0);
}
END_TEST()

FN_TEST(memory_write_after_mprotect)
{
	// Write by memory after an `mprotect`.
	//
	// This is to ensure that an `mprotect` will not mark a page
	// as writable unless it is also marked as dirty.
	TEST_SUCC(mprotect(mapped_buf, PAGE_SIZE,
			   PROT_READ | PROT_WRITE | PROT_EXEC));
	strcpy(&mapped_buf[9], "protected");
	strcpy(&expected_buf[9], "protected");

	// Barrier.
	asm volatile("" : : : "memory");

	// Read by syscall.
	TEST_RES(pread(fd, read_buf, PAGE_SIZE, 0), _ret == PAGE_SIZE);
	TEST_RES(memcmp(read_buf, expected_buf, PAGE_SIZE), _ret == 0);

	// Read by memory.
	TEST_RES(memcmp(mapped_buf, expected_buf, PAGE_SIZE), _ret == 0);
}
END_TEST()

// Truncating the file will remove the mapped pages. Note that this also
// includes COWed pages in private mappings.

static int check_page_does_not_exist(char *ptr)
{
	pid_t child;
	int status;

	child = CHECK(fork());

	if (child == 0) {
		*(volatile char *)ptr = 1;
		exit(EXIT_FAILURE);
	}

	CHECK_WITH(wait(&status), _ret == child);

	if (WIFSIGNALED(status) &&
	// FIXME: Asterinas will send a wrong signal for a nonexistent page.
#ifdef __asterinas__
	    WTERMSIG(status) == SIGSEGV
#else
	    WTERMSIG(status) == SIGBUS
#endif
	)
		return 0;

	errno = EFAULT;
	return -1;
}

FN_TEST(truncate_mapped_pages)
{
	strcpy(&mapped_buf[PAGE_SIZE], "hello, world");

	// After truncation, a shared page must disappear.
	TEST_SUCC(ftruncate(fd, PAGE_SIZE));
	asm volatile("" : : : "memory");
	TEST_SUCC(check_page_does_not_exist(&mapped_buf[PAGE_SIZE]));

	// The new page should be a zero page.
	TEST_SUCC(ftruncate(fd, PAGE_SIZE * 2));
	asm volatile("" : : : "memory");
	TEST_RES(mapped_buf[PAGE_SIZE], _ret == 0);
}
END_TEST()

FN_TEST(truncate_cowed_pages)
{
	int fd;
	char *private_buf;

	fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
	private_buf =
		TEST_SUCC(mmap(NULL, PAGE_SIZE * 2, PROT_READ | PROT_WRITE,
			       MAP_PRIVATE, fd, 0));

	// The page shares. We haven't written anything yet, so there are no COWs.
	TEST_RES(pwrite(fd, "hello", 6, PAGE_SIZE), _ret == 6);
	asm volatile("" : : : "memory");
	TEST_RES(strcmp(&private_buf[PAGE_SIZE], "hello"), _ret == 0);

	// This ensures that the page shares.
	TEST_RES(pwrite(fd, "world", 6, PAGE_SIZE), _ret == 6);
	asm volatile("" : : : "memory");
	TEST_RES(strcmp(&private_buf[PAGE_SIZE], "world"), _ret == 0);

	// Now COWs happen.
	strcpy(&private_buf[PAGE_SIZE], "deadbeef");
	TEST_RES(pwrite(fd, "foofoobarbar", 12, PAGE_SIZE), _ret == 12);
	asm volatile("" : : : "memory");
	TEST_RES(strcmp(&private_buf[PAGE_SIZE], "deadbeef"), _ret == 0);

	// After truncation, a COWed page should also disappear according to Linux.
	TEST_SUCC(ftruncate(fd, PAGE_SIZE));
	asm volatile("" : : : "memory");
	// FIXME: In Asterinas, we build reverse mappings only for shared mappings.
#ifdef __asterinas__
	TEST_RES(strcmp(&private_buf[PAGE_SIZE], "deadbeef"), _ret == 0);
#else
	TEST_SUCC(check_page_does_not_exist(&private_buf[PAGE_SIZE]));

	// The new page should be a zero page.
	TEST_SUCC(ftruncate(fd, PAGE_SIZE * 2));
	asm volatile("" : : : "memory");
	TEST_RES(private_buf[PAGE_SIZE], _ret == 0);
#endif

	TEST_SUCC(munmap(private_buf, PAGE_SIZE * 2));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(munmap(mapped_buf, PAGE_SIZE * 2));
	CHECK(close(fd));
	CHECK(unlink(TEST_FILE));
}
END_SETUP()
