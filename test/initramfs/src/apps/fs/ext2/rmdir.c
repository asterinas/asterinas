// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

#define NON_EMPTY_DIR "/ext2/test_non_empty_dir"
#define NON_EMPTY_CHILD "/ext2/test_non_empty_dir/test.txt"

static void remove_file_if_exists(const char *path)
{
	if (unlink(path) == -1 && errno != ENOENT) {
		fprintf(stderr, "cleanup failed: unlink(%s): %s\n", path,
			strerror(errno));
		exit(EXIT_FAILURE);
	}
}

static void remove_dir_if_exists(const char *path)
{
	if (rmdir(path) == -1 && errno != ENOENT) {
		fprintf(stderr, "cleanup failed: rmdir(%s): %s\n", path,
			strerror(errno));
		exit(EXIT_FAILURE);
	}
}

FN_TEST(rmdir_failed_non_empty_dir)
{
	remove_file_if_exists(NON_EMPTY_CHILD);
	remove_dir_if_exists(NON_EMPTY_DIR);

	TEST_SUCC(mkdir(NON_EMPTY_DIR, 0777));
	TEST_SUCC(open(NON_EMPTY_CHILD, O_CREAT | O_WRONLY, 0666));

	TEST_ERRNO(rmdir(NON_EMPTY_DIR), ENOTEMPTY);

	TEST_SUCC(unlink(NON_EMPTY_CHILD));
	TEST_SUCC(rmdir(NON_EMPTY_DIR));
}
END_TEST()

FN_TEST(unlink_failed_on_directory)
{
	remove_dir_if_exists(NON_EMPTY_DIR);

	TEST_SUCC(mkdir(NON_EMPTY_DIR, 0777));
	TEST_ERRNO(unlink(NON_EMPTY_DIR), EISDIR);

	TEST_SUCC(rmdir(NON_EMPTY_DIR));
}
END_TEST()