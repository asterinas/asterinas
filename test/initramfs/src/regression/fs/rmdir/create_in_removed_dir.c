// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

static char dir_path[PATH_MAX];
static char file_path[PATH_MAX];
static char renamed_path[PATH_MAX];
static char replaced_path[PATH_MAX];
static int dirfd;

FN_SETUP(remove_dir)
{
	const char *base = getenv("TEST_TMPDIR");

	if (!base) {
		base = "/tmp";
	}
	CHECK_WITH(snprintf(dir_path, sizeof(dir_path), "%s/removed_dir", base),
		   _ret > 0 && (size_t)_ret < sizeof(dir_path));
	CHECK_WITH(snprintf(file_path, sizeof(file_path), "%s/removed_dir_file",
			    base),
		   _ret > 0 && (size_t)_ret < sizeof(file_path));
	CHECK_WITH(snprintf(renamed_path, sizeof(renamed_path),
			    "%s/renamed_dir", base),
		   _ret > 0 && (size_t)_ret < sizeof(renamed_path));
	CHECK_WITH(snprintf(replaced_path, sizeof(replaced_path),
			    "%s/replaced_dir", base),
		   _ret > 0 && (size_t)_ret < sizeof(replaced_path));
	CHECK_WITH(rmdir(dir_path), _ret == 0 || errno == ENOENT);
	CHECK_WITH(unlink(file_path), _ret == 0 || errno == ENOENT);
	CHECK_WITH(rmdir(renamed_path), _ret == 0 || errno == ENOENT);
	CHECK_WITH(rmdir(replaced_path), _ret == 0 || errno == ENOENT);

	CHECK(close(CHECK(open(file_path, O_CREAT | O_WRONLY, 0644))));
	CHECK(mkdir(dir_path, 0755));
	dirfd = CHECK(open(dir_path, O_RDONLY | O_DIRECTORY));
	CHECK(rmdir(dir_path));
}
END_SETUP()

FN_TEST(create_entries_in_removed_dir)
{
	struct stat st;

	TEST_ERRNO(mkdirat(dirfd, "dir", 0755), ENOENT);
	TEST_ERRNO(openat(dirfd, "file", O_CREAT | O_RDWR, 0644), ENOENT);
	TEST_ERRNO(symlinkat("target", dirfd, "symlink"), ENOENT);
	TEST_ERRNO(mknodat(dirfd, "fifo", S_IFIFO | 0644, 0), ENOENT);
	TEST_ERRNO(linkat(AT_FDCWD, file_path, dirfd, "link", 0), ENOENT);
	TEST_ERRNO(renameat(AT_FDCWD, file_path, dirfd, "renamed"), ENOENT);

	TEST_RES(fstat(dirfd, &st), st.st_nlink == 0);
	TEST_SUCC(access(file_path, F_OK));
}
END_TEST()

FN_TEST(create_entries_in_removed_cwd)
{
	TEST_SUCC(fchdir(dirfd));
	TEST_ERRNO(open("file", O_CREAT | O_RDWR, 0644), ENOENT);
	TEST_ERRNO(mkdir("dir", 0755), ENOENT);
	TEST_SUCC(chdir("/"));
}
END_TEST()

FN_TEST(create_entries_in_dir_replaced_by_rename)
{
	struct stat st;
	int fd;

	TEST_SUCC(mkdir(renamed_path, 0755));
	TEST_SUCC(mkdir(replaced_path, 0755));
	fd = TEST_SUCC(open(replaced_path, O_RDONLY | O_DIRECTORY));
	TEST_SUCC(rename(renamed_path, replaced_path));
	TEST_RES(fstat(fd, &st), st.st_nlink == 0);

	TEST_ERRNO(mkdirat(fd, "dir", 0755), ENOENT);
	TEST_ERRNO(openat(fd, "file", O_CREAT | O_RDWR, 0644), ENOENT);

	TEST_SUCC(close(fd));
	TEST_SUCC(rmdir(replaced_path));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(dirfd));
	CHECK(unlink(file_path));
}
END_SETUP()
