/* SPDX-License-Identifier: MPL-2.0 */

#define _GNU_SOURCE

#include <dirent.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/tmp/aster_tmpfile_test"
#define CROSS_LINK_DIR "/ext2"
#define CROSS_LINK_NAME "cross_mount_tmpfile"
#define LINKED_NAME "linked_file"
#define LINKED_SECOND_NAME "linked_second_file"
#define LINKED_O_EXCL_NAME "linked_o_excl_file"
#define SYMLINK_NAME "tmpfile_symlink"
#define DATA "hello from tmpfile"
#define DATA_LEN (sizeof(DATA) - 1)

#define RAW_O_TMPFILE 020000000

/* O_TMPFILE may not be defined on older glibc. */
#ifndef __O_TMPFILE
#define __O_TMPFILE RAW_O_TMPFILE
#endif

#ifndef O_TMPFILE
#define O_TMPFILE (__O_TMPFILE | O_DIRECTORY)
#endif

#ifndef AT_EMPTY_PATH
#define AT_EMPTY_PATH 0x1000
#endif

static void cleanup_test_files(void)
{
	unlink(TEST_DIR "/" LINKED_NAME);
	unlink(TEST_DIR "/" LINKED_SECOND_NAME);
	unlink(TEST_DIR "/" LINKED_O_EXCL_NAME);
	unlink(TEST_DIR "/" SYMLINK_NAME);
	unlink(CROSS_LINK_DIR "/" CROSS_LINK_NAME);
	rmdir(TEST_DIR);
}

static int timespec_equal(struct timespec left, struct timespec right)
{
	return left.tv_sec == right.tv_sec && left.tv_nsec == right.tv_nsec;
}

static int dir_is_unavailable_or_same_mount(const char *source_path,
					    const char *target_path)
{
	struct stat source_stat;
	struct stat target_stat;

	if (stat(source_path, &source_stat) < 0 ||
	    stat(target_path, &target_stat) < 0) {
		return 1;
	}

	return source_stat.st_dev == target_stat.st_dev;
}

FN_SETUP(prepare)
{
	cleanup_test_files();
	CHECK(mkdir(TEST_DIR, 0755));
}
END_SETUP()

FN_TEST(tmpfile_open_succeeds)
{
	int fd;

	fd = TEST_SUCC(open(TEST_DIR, O_TMPFILE | O_RDWR, 0666));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(tmpfile_open_write_only_succeeds)
{
	int fd;

	fd = TEST_SUCC(open(TEST_DIR, O_TMPFILE | O_WRONLY, 0666));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(tmpfile_open_read_only_returns_einval)
{
	TEST_ERRNO(open(TEST_DIR, O_TMPFILE | O_RDONLY, 0666), EINVAL);
}
END_TEST()

FN_TEST(tmpfile_open_without_o_directory_returns_einval)
{
	TEST_ERRNO(open(TEST_DIR, RAW_O_TMPFILE | O_RDWR, 0666), EINVAL);
}
END_TEST()

FN_TEST(tmpfile_open_with_o_creat_returns_einval)
{
	TEST_ERRNO(open(TEST_DIR, O_TMPFILE | O_RDWR | O_CREAT, 0666), EINVAL);
}
END_TEST()

FN_TEST(tmpfile_open_with_o_path_yields_path_fd)
{
	int fd;
	char buf[1];

	fd = TEST_SUCC(open(TEST_DIR, O_TMPFILE | O_PATH | O_RDWR, 0666));
	TEST_ERRNO(read(fd, buf, sizeof(buf)), EBADF);

	TEST_SUCC(close(fd));
}
END_TEST()

// FIXME: Add `O_TMPFILE` support for ext2.
#ifdef __asterinas__
FN_TEST(tmpfile_open_on_ext2_returns_eopnotsupp)
{
	TEST_ERRNO(open(CROSS_LINK_DIR, O_TMPFILE | O_RDWR, 0666), EOPNOTSUPP);
}
END_TEST()
#endif

FN_TEST(tmpfile_open_with_o_excl_succeeds)
{
	int fd;

	fd = TEST_SUCC(open(TEST_DIR, O_TMPFILE | O_RDWR | O_EXCL, 0666));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(tmpfile_open_symlink_with_o_nofollow_returns_enotdir)
{
	TEST_SUCC(symlink(TEST_DIR, TEST_DIR "/" SYMLINK_NAME));

	TEST_ERRNO(open(TEST_DIR "/" SYMLINK_NAME,
			O_TMPFILE | O_RDWR | O_NOFOLLOW, 0666),
		   ENOTDIR);

	TEST_SUCC(unlink(TEST_DIR "/" SYMLINK_NAME));
}
END_TEST()

FN_TEST(tmpfile_open_non_dir_returns_enotdir)
{
	TEST_ERRNO(open("/dev/null", O_TMPFILE | O_RDWR, 0666), ENOTDIR);
}
END_TEST()

FN_TEST(tmpfile_open_does_not_update_parent_timestamps)
{
	struct stat before;
	struct stat after;
	int fd;

	TEST_SUCC(stat(TEST_DIR, &before));
	fd = TEST_SUCC(open(TEST_DIR, O_TMPFILE | O_RDWR, 0666));
	TEST_SUCC(close(fd));
	TEST_SUCC(stat(TEST_DIR, &after));

	TEST_RES(timespec_equal(after.st_mtim, before.st_mtim), _ret);
	TEST_RES(timespec_equal(after.st_ctim, before.st_ctim), _ret);
}
END_TEST()

FN_TEST(tmpfile_invisible_in_readdir)
{
	DIR *dir;
	int fd;
	int found;
	struct dirent *entry;

	fd = TEST_SUCC(open(TEST_DIR, O_TMPFILE | O_RDWR, 0666));

	dir = TEST_RES(opendir(TEST_DIR), _ret != NULL);

	found = 0;
	while ((entry = readdir(dir)) != NULL) {
		if (strcmp(entry->d_name, ".") != 0 &&
		    strcmp(entry->d_name, "..") != 0) {
			found++;
		}
	}
	TEST_RES(found, found == 0);

	TEST_SUCC(closedir(dir));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(tmpfile_write_and_linkat)
{
	DIR *dir;
	char buf[DATA_LEN];
	int dirfd;
	int fd;
	int found;
	int linked_fd;
	struct dirent *entry;
	struct stat stat_after_link;
	struct stat stat_after_second_link;

	fd = TEST_SUCC(open(TEST_DIR, O_TMPFILE | O_RDWR, 0666));

	TEST_RES(write(fd, DATA, DATA_LEN), _ret == DATA_LEN);

	TEST_RES(pread(fd, buf, sizeof(buf), 0), _ret == DATA_LEN);
	TEST_RES(memcmp(buf, DATA, DATA_LEN), _ret == 0);

	dirfd = TEST_SUCC(open(TEST_DIR, O_RDONLY | O_DIRECTORY));
	TEST_SUCC(linkat(fd, "", dirfd, LINKED_NAME, AT_EMPTY_PATH));

	TEST_RES(fstat(fd, &stat_after_link), stat_after_link.st_nlink == 1);

	dir = TEST_RES(opendir(TEST_DIR), _ret != NULL);
	found = 0;
	while ((entry = readdir(dir)) != NULL) {
		found += strcmp(entry->d_name, LINKED_NAME) == 0;
	}
	TEST_RES(found, found == 1);
	TEST_SUCC(closedir(dir));

	TEST_SUCC(link(TEST_DIR "/" LINKED_NAME,
		       TEST_DIR "/" LINKED_SECOND_NAME));
	TEST_RES(fstat(fd, &stat_after_second_link),
		 stat_after_second_link.st_nlink == 2);

	linked_fd = TEST_SUCC(open(TEST_DIR "/" LINKED_NAME, O_RDONLY));
	TEST_RES(pread(linked_fd, buf, sizeof(buf), 0), _ret == DATA_LEN);
	TEST_RES(memcmp(buf, DATA, DATA_LEN), _ret == 0);

	TEST_SUCC(close(linked_fd));
	TEST_SUCC(close(dirfd));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(tmpfile_open_with_o_excl_cannot_be_linked)
{
	int fd;
	int dirfd;

	fd = TEST_SUCC(open(TEST_DIR, O_TMPFILE | O_RDWR | O_EXCL, 0666));
	dirfd = TEST_SUCC(open(TEST_DIR, O_RDONLY | O_DIRECTORY));

	TEST_ERRNO(linkat(fd, "", dirfd, LINKED_O_EXCL_NAME, AT_EMPTY_PATH),
		   ENOENT);

	TEST_SUCC(close(dirfd));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(tmpfile_linkat_cross_mount_returns_exdev)
{
	int fd;
	int tmpfd;

#ifdef __asterinas__
	TEST_RES(dir_is_unavailable_or_same_mount(TEST_DIR, CROSS_LINK_DIR),
		 _ret == 0);
#else
	SKIP_TEST_IF(
		dir_is_unavailable_or_same_mount(TEST_DIR, CROSS_LINK_DIR));
#endif

	fd = TEST_SUCC(open(TEST_DIR, O_TMPFILE | O_RDWR, 0666));
	tmpfd = TEST_SUCC(open(CROSS_LINK_DIR, O_RDONLY | O_DIRECTORY));

	TEST_ERRNO(linkat(fd, "", tmpfd, CROSS_LINK_NAME, AT_EMPTY_PATH),
		   EXDEV);

	TEST_SUCC(close(tmpfd));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(cleanup)
{
	cleanup_test_files();
}
END_SETUP()
