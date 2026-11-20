// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/inotify.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define WATCH_DIR "/tmp/inotify_fdinfo_test"
#define UNCACHED_FILE "/ext2/inotify_fdinfo_uncached"

#define PAGE_SIZE 4096

// A regression test for the bug described in #4167.
FN_TEST(fdinfo_read_with_faulting_buffer)
{
	static char page[PAGE_SIZE] __attribute__((aligned(PAGE_SIZE)));

	/*
	 * The mapping faults on access and sleeps while reading the disk, because
	 * its pages belong to an ext2 file whose data is not in the page cache:
	 * `O_DIRECT` writes bypass the page cache.
	 */
	int file_fd =
		CHECK(open(UNCACHED_FILE, O_RDWR | O_CREAT | O_DIRECT, 0644));
	CHECK_WITH(pwrite(file_fd, page, PAGE_SIZE, 0), _ret == PAGE_SIZE);
	CHECK_WITH(pwrite(file_fd, page, PAGE_SIZE, PAGE_SIZE),
		   _ret == PAGE_SIZE);
	char *buf = CHECK_WITH(mmap(NULL, 2 * PAGE_SIZE, PROT_READ | PROT_WRITE,
				    MAP_SHARED, file_fd, 0),
			       _ret != MAP_FAILED);
	CHECK(close(file_fd));

	TEST_SUCC(mkdir(WATCH_DIR, 0755));
	int inotify_fd = TEST_SUCC(inotify_init1(0));
	int wd = TEST_SUCC(inotify_add_watch(inotify_fd, WATCH_DIR, IN_CREATE));

	char fdinfo_path[64];
	snprintf(fdinfo_path, sizeof(fdinfo_path), "/proc/self/fdinfo/%d",
		 inotify_fd);
	char watch_line[64];
	snprintf(watch_line, sizeof(watch_line), "inotify wd:%d ", wd);

	/*
	 * The read writes the header lines before the watch line. Read once into
	 * the first page of the mapping so that the page is present, then read
	 * again from just before it ends, so that the first byte written after it
	 * belongs to the watch line. The old code held the watch-map lock while
	 * formatting the watch entry.
	 */
	int fdinfo_fd = TEST_SUCC(open(fdinfo_path, O_RDONLY));
	int len = CHECK_WITH(read(fdinfo_fd, buf, PAGE_SIZE),
			     _ret > 0 && _ret < PAGE_SIZE);
	char *watch = CHECK_WITH(memmem(buf, (size_t)len, watch_line,
					strlen(watch_line)),
				 _ret != NULL);
	char *crossing = buf + PAGE_SIZE - (watch - buf);
	TEST_SUCC(close(fdinfo_fd));

	fdinfo_fd = TEST_SUCC(open(fdinfo_path, O_RDONLY));
	TEST_RES(read(fdinfo_fd, crossing, PAGE_SIZE),
		 _ret > 0 && _ret < PAGE_SIZE &&
			 memmem(crossing, (size_t)_ret, watch_line,
				strlen(watch_line)) != NULL);

	TEST_SUCC(close(fdinfo_fd));
	TEST_SUCC(close(inotify_fd));
	TEST_SUCC(rmdir(WATCH_DIR));
	CHECK(munmap(buf, 2 * PAGE_SIZE));
	CHECK(unlink(UNCACHED_FILE));
}
END_TEST()
