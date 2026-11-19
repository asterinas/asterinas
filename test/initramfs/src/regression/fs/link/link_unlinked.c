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

static char source[PATH_MAX];
static char other[PATH_MAX];
static char target[PATH_MAX];

static void make_path(char *buffer, const char *dir, const char *name)
{
	CHECK_WITH(snprintf(buffer, PATH_MAX, "%s/%s", dir, name),
		   _ret > 0 && _ret < PATH_MAX);
}

static void make_proc_fd_path(char *buffer, int fd)
{
	CHECK_WITH(snprintf(buffer, PATH_MAX, "/proc/self/fd/%d", fd),
		   _ret > 0 && _ret < PATH_MAX);
}

static void cleanup_test_files(void)
{
	CHECK_WITH(unlink(source), _ret == 0 || errno == ENOENT);
	CHECK_WITH(unlink(other), _ret == 0 || errno == ENOENT);
	CHECK_WITH(unlink(target), _ret == 0 || errno == ENOENT);
}

FN_SETUP(set_paths)
{
	const char *dir = getenv("TEST_TMPDIR");

	if (!dir) {
		dir = "/tmp";
	}
	make_path(source, dir, "link_unlinked_source");
	make_path(other, dir, "link_unlinked_other");
	make_path(target, dir, "link_unlinked_target");
	cleanup_test_files();
}
END_SETUP()

FN_TEST(link_file_with_a_link)
{
	char proc_fd[PATH_MAX];
	int fd;

	fd = TEST_SUCC(open(source, O_CREAT | O_EXCL | O_RDWR, 0600));
	make_proc_fd_path(proc_fd, fd);

	TEST_SUCC(
		linkat(AT_FDCWD, proc_fd, AT_FDCWD, target, AT_SYMLINK_FOLLOW));
	TEST_SUCC(unlink(target));

	TEST_SUCC(unlink(source));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(link_unlinked_file)
{
	char proc_fd[PATH_MAX];
	struct stat st;
	int fd;

	fd = TEST_SUCC(open(source, O_CREAT | O_EXCL | O_RDWR, 0600));
	make_proc_fd_path(proc_fd, fd);
	TEST_SUCC(unlink(source));
	TEST_RES(fstat(fd, &st), st.st_nlink == 0);

	TEST_ERRNO(linkat(AT_FDCWD, proc_fd, AT_FDCWD, target,
			  AT_SYMLINK_FOLLOW),
		   ENOENT);
	TEST_ERRNO(linkat(fd, "", AT_FDCWD, target, AT_EMPTY_PATH), ENOENT);
	TEST_ERRNO(access(target, F_OK), ENOENT);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(link_file_replaced_by_rename)
{
	char proc_fd[PATH_MAX];
	struct stat st;
	int fd;
	int other_fd;

	fd = TEST_SUCC(open(source, O_CREAT | O_EXCL | O_RDWR, 0600));
	make_proc_fd_path(proc_fd, fd);
	other_fd = TEST_SUCC(open(other, O_CREAT | O_EXCL | O_WRONLY, 0600));
	TEST_SUCC(close(other_fd));
	TEST_SUCC(rename(other, source));
	TEST_RES(fstat(fd, &st), st.st_nlink == 0);

	TEST_ERRNO(linkat(AT_FDCWD, proc_fd, AT_FDCWD, target,
			  AT_SYMLINK_FOLLOW),
		   ENOENT);
	TEST_ERRNO(linkat(fd, "", AT_FDCWD, target, AT_EMPTY_PATH), ENOENT);
	TEST_ERRNO(access(target, F_OK), ENOENT);

	TEST_SUCC(unlink(source));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(cleanup)
{
	cleanup_test_files();
}
END_SETUP()
