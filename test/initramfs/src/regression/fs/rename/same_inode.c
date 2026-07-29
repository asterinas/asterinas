// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <stdio.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define RAMFS_BASE_DIR "/tmp/rename_same_inode"
#define RAMFS_CROSS_DIR_BASE_DIR "/tmp/rename_same_inode_cross_dir"
#define RAMFS_SOURCE_DIR RAMFS_CROSS_DIR_BASE_DIR "/source"
#define RAMFS_TARGET_DIR RAMFS_CROSS_DIR_BASE_DIR "/target"
#define EXT2_BASE_DIR "/ext2/rename_same_inode"

static void make_path(char *buffer, size_t size, const char *base,
		      const char *name)
{
	int length = snprintf(buffer, size, "%s/%s", base, name);
	CHECK_WITH(length, _ret > 0 && (size_t)_ret < size);
}

static void cleanup_test_files(const char *base)
{
	char path[256];

	make_path(path, sizeof(path), base, "a");
	CHECK_WITH(unlink(path), _ret == 0 || errno == ENOENT);
	make_path(path, sizeof(path), base, "b");
	CHECK_WITH(unlink(path), _ret == 0 || errno == ENOENT);
	CHECK_WITH(rmdir(base), _ret == 0 || errno == ENOENT);
}

static int verify_two_hardlinks(const char *path_a, const char *path_b)
{
	struct stat stat_a;
	struct stat stat_b;
	if (stat(path_a, &stat_a) < 0 || stat(path_b, &stat_b) < 0) {
		return -1;
	}
	if (stat_a.st_ino != stat_b.st_ino || stat_a.st_nlink != 2 ||
	    stat_b.st_nlink != 2) {
		errno = EIO;
		return -1;
	}

	return 0;
}

static int verify_same_dir_rename(const char *base)
{
	char path_a[256];
	char path_b[256];
	make_path(path_a, sizeof(path_a), base, "a");
	make_path(path_b, sizeof(path_b), base, "b");

	if (mkdir(base, 0755) < 0) {
		return -1;
	}
	int fd = open(path_a, O_CREAT | O_EXCL | O_WRONLY, 0644);
	if (fd < 0) {
		return -1;
	}
	if (close(fd) < 0 || link(path_a, path_b) < 0 ||
	    rename(path_a, path_b) < 0) {
		return -1;
	}

	return verify_two_hardlinks(path_a, path_b);
}

static void cleanup_cross_dir_test_files(void)
{
	char path_a[256];
	char path_b[256];
	make_path(path_a, sizeof(path_a), RAMFS_SOURCE_DIR, "a");
	make_path(path_b, sizeof(path_b), RAMFS_TARGET_DIR, "b");

	CHECK_WITH(unlink(path_a), _ret == 0 || errno == ENOENT);
	CHECK_WITH(unlink(path_b), _ret == 0 || errno == ENOENT);
	CHECK_WITH(rmdir(RAMFS_SOURCE_DIR), _ret == 0 || errno == ENOENT);
	CHECK_WITH(rmdir(RAMFS_TARGET_DIR), _ret == 0 || errno == ENOENT);
	CHECK_WITH(rmdir(RAMFS_CROSS_DIR_BASE_DIR),
		   _ret == 0 || errno == ENOENT);
}

static int verify_cross_dir_rename(void)
{
	char path_a[256];
	char path_b[256];
	make_path(path_a, sizeof(path_a), RAMFS_SOURCE_DIR, "a");
	make_path(path_b, sizeof(path_b), RAMFS_TARGET_DIR, "b");

	if (mkdir(RAMFS_CROSS_DIR_BASE_DIR, 0755) < 0 ||
	    mkdir(RAMFS_SOURCE_DIR, 0755) < 0 ||
	    mkdir(RAMFS_TARGET_DIR, 0755) < 0) {
		return -1;
	}
	int fd = open(path_a, O_CREAT | O_EXCL | O_WRONLY, 0644);
	if (fd < 0) {
		return -1;
	}
	if (close(fd) < 0 || link(path_a, path_b) < 0 ||
	    rename(path_a, path_b) < 0) {
		return -1;
	}

	return verify_two_hardlinks(path_a, path_b);
}

FN_TEST(rename_between_hardlinks_is_noop_on_ramfs)
{
	cleanup_test_files(RAMFS_BASE_DIR);
	TEST_SUCC(verify_same_dir_rename(RAMFS_BASE_DIR));
	cleanup_test_files(RAMFS_BASE_DIR);
}
END_TEST()

FN_TEST(rename_between_cross_dir_hardlinks_is_noop_on_ramfs)
{
	cleanup_cross_dir_test_files();
	TEST_SUCC(verify_cross_dir_rename());
	cleanup_cross_dir_test_files();
}
END_TEST()

FN_TEST(rename_between_hardlinks_is_noop_on_ext2)
{
	SKIP_TEST_IF(access("/ext2", F_OK) < 0);

	cleanup_test_files(EXT2_BASE_DIR);
	TEST_SUCC(verify_same_dir_rename(EXT2_BASE_DIR));
	cleanup_test_files(EXT2_BASE_DIR);
}
END_TEST()
