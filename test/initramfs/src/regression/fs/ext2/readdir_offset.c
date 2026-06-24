// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/ext2/readdir_offset_test"
#define TEST_ENTRY_COUNT 32
#define TEST_NAME_PREFIX "entry"

struct linux_dirent64 {
	uint64_t d_ino;
	int64_t d_off;
	unsigned short d_reclen;
	unsigned char d_type;
	char d_name[];
};

static void format_entry_path(char *path, size_t path_len, int idx)
{
	CHECK_WITH(snprintf(path, path_len, "%s/%s%02d", TEST_DIR,
			    TEST_NAME_PREFIX, idx),
		   _ret > 0 && (size_t)_ret < path_len);
}

static void cleanup_test_dir(void)
{
	char path[64];

	for (int i = 0; i < TEST_ENTRY_COUNT; i++) {
		format_entry_path(path, sizeof(path), i);
		CHECK_WITH(unlink(path), _ret == 0 || errno == ENOENT);
	}
	CHECK_WITH(rmdir(TEST_DIR), _ret == 0 || errno == ENOENT);
}

static int is_test_entry(const char *name)
{
	return strncmp(name, TEST_NAME_PREFIX, strlen(TEST_NAME_PREFIX)) == 0;
}

static int count_test_entries_with_unique_offsets(char *buffer, int nread)
{
	int64_t offsets[TEST_ENTRY_COUNT];
	int found = 0;

	for (int pos = 0; pos < nread;) {
		struct linux_dirent64 *dirent =
			(struct linux_dirent64 *)(buffer + pos);

		if (dirent->d_reclen == 0 || pos + dirent->d_reclen > nread) {
			errno = EINVAL;
			return -1;
		}

		if (!is_test_entry(dirent->d_name)) {
			pos += dirent->d_reclen;
			continue;
		}

		if (found == TEST_ENTRY_COUNT) {
			errno = EOVERFLOW;
			return -1;
		}
		for (int i = 0; i < found; i++) {
			if (offsets[i] == dirent->d_off) {
				errno = EEXIST;
				return -1;
			}
		}

		offsets[found++] = dirent->d_off;
		pos += dirent->d_reclen;
	}

	return found;
}

FN_SETUP(cleanup_before_test)
{
	cleanup_test_dir();
}
END_SETUP()

FN_TEST(ext2_readdir_offsets_are_unique)
{
	char path[64];
	char buffer[8192];

	TEST_SUCC(mkdir(TEST_DIR, 0755));
	for (int i = 0; i < TEST_ENTRY_COUNT; i++) {
		format_entry_path(path, sizeof(path), i);
		int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
		TEST_SUCC(close(fd));
	}

	int dir_fd = TEST_SUCC(open(TEST_DIR, O_RDONLY | O_DIRECTORY));
	int nread = TEST_RES(syscall(SYS_getdents64, dir_fd, buffer,
				     sizeof(buffer)),
			     _ret > 0);
	TEST_RES(count_test_entries_with_unique_offsets(buffer, nread),
		 _ret == TEST_ENTRY_COUNT);

	TEST_SUCC(close(dir_fd));
	cleanup_test_dir();
}
END_TEST()
