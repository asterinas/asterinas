/* SPDX-License-Identifier: MPL-2.0 */

#define _GNU_SOURCE

#include <unistd.h>
#include <sys/mman.h>
#include <sys/fcntl.h>

#include "../../common/test.h"

#define PAGE_SIZE 4096

#define TEST_FILE "/ext2/short_rw"
#define TEST_DATA "abcdefg"

FN_TEST(short_write_should_not_leak_uninit_cache_page)
{
	char *buf;
	int fd;

	buf = TEST_SUCC(mmap(NULL, PAGE_SIZE * 2, PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));
	TEST_SUCC(munmap(buf + PAGE_SIZE, PAGE_SIZE));

	// Write a page to the disk via O_DIRECT.
	fd = TEST_SUCC(
		open(TEST_FILE, O_WRONLY | O_DIRECT | O_CREAT | O_TRUNC, 0644));
	strcpy(buf, TEST_DATA);
	TEST_RES(write(fd, buf, PAGE_SIZE), _ret == 4096);
	TEST_SUCC(close(fd));

	memset(buf, 0, PAGE_SIZE);

	fd = TEST_SUCC(open(TEST_FILE, O_RDWR));

	// If it succeeds, it will overwrite a page in the page cache.
	// As an optimization, the page does not need to be loaded from the disk.
	// However, if it is a short write and fails, the page will be left uninitialized.
	buf[PAGE_SIZE - 1] = 'X';
	TEST_ERRNO(write(fd, buf + PAGE_SIZE - 1, PAGE_SIZE), EFAULT);

	// No uninitialized data should leak to user space.
	// The user should see the original data.
	TEST_RES(lseek(fd, 0, SEEK_CUR), _ret == 0);
	TEST_RES(read(fd, buf, PAGE_SIZE),
		 _ret == PAGE_SIZE && strcmp(buf, TEST_DATA) == 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE));

	TEST_SUCC(munmap(buf, PAGE_SIZE));
}
END_TEST()
