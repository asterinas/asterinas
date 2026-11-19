// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <dirent.h>
#include <fcntl.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/tmp/ramfs_readdir_test"
#define TEST_FILE TEST_DIR "/file"

#define PAGE_SIZE 4096
#define UNCACHED_FILE "/ext2/ramfs_readdir_uncached"

FN_TEST(seekdir_resumes_after_entry)
{
	const char *entries[] = { ".", "..", "file" };
	const size_t nr_entries = sizeof(entries) / sizeof(entries[0]);
	long offsets[nr_entries];

	TEST_SUCC(mkdir(TEST_DIR, 0755));
	int fd = TEST_SUCC(creat(TEST_FILE, 0644));
	TEST_SUCC(close(fd));

	DIR *dir = TEST_SUCC(opendir(TEST_DIR));
	for (size_t i = 0; i < nr_entries; ++i) {
		TEST_RES(readdir(dir),
			 _ret != NULL && strcmp(_ret->d_name, entries[i]) == 0);
		offsets[i] = TEST_RES(telldir(dir), _ret >= 0);
	}

	for (size_t i = 0; i < nr_entries; ++i) {
		seekdir(dir, offsets[i]);
		TEST_RES(readdir(dir),
			 i + 1 == nr_entries ?
				 _ret == NULL :
				 _ret != NULL && strcmp(_ret->d_name,
							entries[i + 1]) == 0);
	}

	TEST_SUCC(closedir(dir));
	TEST_SUCC(unlink(TEST_FILE));
	TEST_SUCC(rmdir(TEST_DIR));
}
END_TEST()

FN_TEST(getdents_with_faulting_buffer)
{
	static char page[PAGE_SIZE] __attribute__((aligned(PAGE_SIZE)));

	/*
	 * The buffer faults on its first access and sleeps while reading the disk,
	 * because it maps a page of an ext2 file whose data is not in the page
	 * cache: `O_DIRECT` writes bypass the page cache.
	 */
	int buf_fd = TEST_SUCC(
		open(UNCACHED_FILE, O_RDWR | O_CREAT | O_DIRECT, 0644));
	TEST_RES(pwrite(buf_fd, page, PAGE_SIZE, 0), _ret == PAGE_SIZE);
	char *buf = TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
				   MAP_SHARED, buf_fd, 0));
	TEST_SUCC(close(buf_fd));

	TEST_SUCC(mkdir(TEST_DIR, 0755));
	int dir_fd = TEST_SUCC(open(TEST_DIR, O_RDONLY | O_DIRECTORY));

	TEST_RES(syscall(SYS_getdents64, dir_fd, buf, PAGE_SIZE), _ret > 0);

	TEST_SUCC(close(dir_fd));
	TEST_SUCC(rmdir(TEST_DIR));

	TEST_SUCC(munmap(buf, PAGE_SIZE));
	TEST_SUCC(unlink(UNCACHED_FILE));
}
END_TEST()
