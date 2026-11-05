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

static char base[PATH_MAX];

static void make_path(char *buffer, const char *name)
{
	CHECK_WITH(snprintf(buffer, PATH_MAX, "%s/%s", base, name),
		   _ret > 0 && _ret < PATH_MAX);
}

static void cleanup_test_files(void)
{
	static const char *const files[] = { "old_file", "new_file",
					     "sub/old_file" };
	static const char *const dirs[] = { "old_dir", "new_dir", "sub" };
	char path[PATH_MAX];

	for (size_t i = 0; i < sizeof(files) / sizeof(files[0]); i++) {
		make_path(path, files[i]);
		CHECK_WITH(unlink(path), _ret == 0 || errno == ENOENT);
	}
	for (size_t i = 0; i < sizeof(dirs) / sizeof(dirs[0]); i++) {
		make_path(path, dirs[i]);
		CHECK_WITH(rmdir(path), _ret == 0 || errno == ENOENT);
	}
	CHECK_WITH(rmdir(base), _ret == 0 || errno == ENOENT);
}

FN_SETUP(create_base_dir)
{
	const char *dir = getenv("TEST_TMPDIR");
	char path[PATH_MAX];

	if (!dir) {
		dir = "/tmp";
	}
	CHECK_WITH(snprintf(base, sizeof(base), "%s/rename_replaced", dir),
		   _ret > 0 && (size_t)_ret < sizeof(base));
	cleanup_test_files();

	CHECK(mkdir(base, 0755));
	make_path(path, "sub");
	CHECK(mkdir(path, 0755));
}
END_SETUP()

FN_TEST(replace_file)
{
	char old_path[PATH_MAX];
	char other_dir_path[PATH_MAX];
	char new_path[PATH_MAX];
	struct stat st;
	int fd;

	make_path(old_path, "old_file");
	make_path(other_dir_path, "sub/old_file");
	make_path(new_path, "new_file");
	fd = TEST_SUCC(open(old_path, O_CREAT | O_EXCL | O_WRONLY, 0600));
	TEST_SUCC(close(fd));
	fd = TEST_SUCC(open(other_dir_path, O_CREAT | O_EXCL | O_WRONLY, 0600));
	TEST_SUCC(close(fd));

	// Replace a file with one from the same directory.
	fd = TEST_SUCC(open(new_path, O_CREAT | O_EXCL | O_RDWR, 0600));
	TEST_SUCC(rename(old_path, new_path));
	TEST_RES(fstat(fd, &st), st.st_nlink == 0);
	TEST_SUCC(close(fd));

	// Replace a file with one from another directory.
	fd = TEST_SUCC(open(new_path, O_RDWR));
	TEST_SUCC(rename(other_dir_path, new_path));
	TEST_RES(fstat(fd, &st), st.st_nlink == 0);
	TEST_SUCC(close(fd));

	TEST_SUCC(unlink(new_path));
}
END_TEST()

FN_TEST(replace_dir)
{
	char old_path[PATH_MAX];
	char new_path[PATH_MAX];
	struct stat st;
	int fd;

	make_path(old_path, "old_dir");
	make_path(new_path, "new_dir");
	TEST_SUCC(mkdir(old_path, 0755));
	TEST_SUCC(mkdir(new_path, 0755));
	fd = TEST_SUCC(open(new_path, O_RDONLY | O_DIRECTORY));

	TEST_SUCC(rename(old_path, new_path));
	TEST_RES(fstat(fd, &st), st.st_nlink == 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(rmdir(new_path));
}
END_TEST()

FN_SETUP(cleanup)
{
	cleanup_test_files();
}
END_SETUP()
