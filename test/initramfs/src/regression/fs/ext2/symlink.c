// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ext2/ext2_symlink_test"

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

static void unlink_if_exists(const char *path)
{
	CHECK_WITH(unlink(path), _ret == 0 || errno == ENOENT);
}

static void rmdir_if_exists(const char *path)
{
	CHECK_WITH(rmdir(path), _ret == 0 || errno == ENOENT);
}

FN_SETUP(prepare_base_dir)
{
	ensure_dir(BASE_DIR);
}
END_SETUP()

FN_TEST(symlink_short_roundtrip)
{
	const char *file = BASE_DIR "/short_file";
	const char *link = BASE_DIR "/short_link";
	const char *data = "hello symlink";

	int fd = TEST_SUCC(open(file, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(write(fd, data, strlen(data)));
	TEST_SUCC(close(fd));

	TEST_SUCC(symlink(file, link));

	char buf[PATH_MAX];
	TEST_RES(readlink(link, buf, sizeof(buf)),
		 _ret == (ssize_t)strlen(file));
	buf[strlen(file)] = '\0';
	TEST_RES(strcmp(buf, file), _ret == 0);

	// Open via symlink and verify content
	char rbuf[64];
	fd = TEST_SUCC(open(link, O_RDONLY));
	TEST_RES(read(fd, rbuf, sizeof(rbuf)), _ret == (ssize_t)strlen(data));
	rbuf[strlen(data)] = '\0';
	TEST_RES(strcmp(rbuf, data), _ret == 0);
	TEST_SUCC(close(fd));

	unlink_if_exists(link);
	unlink_if_exists(file);
}
END_TEST()

FN_TEST(symlink_long_roundtrip)
{
	// Build a deeply nested path exceeding 60 chars to trigger slow symlink
	const char *dirs[] = {
		BASE_DIR "/aaaa",
		BASE_DIR "/aaaa/bbbb",
		BASE_DIR "/aaaa/bbbb/cccc",
		BASE_DIR "/aaaa/bbbb/cccc/dddd",
		BASE_DIR "/aaaa/bbbb/cccc/dddd/eeee",
		BASE_DIR "/aaaa/bbbb/cccc/dddd/eeee/ffff",
		BASE_DIR "/aaaa/bbbb/cccc/dddd/eeee/ffff/gggg",
	};
	const char *target = BASE_DIR
		"/aaaa/bbbb/cccc/dddd/eeee/ffff/gggg/target_file";
	const char *link = BASE_DIR "/long_link";

	for (int i = 0; i < (int)(sizeof(dirs) / sizeof(dirs[0])); i++)
		ensure_dir(dirs[i]);

	int fd = TEST_SUCC(open(target, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	// Verify the target path is indeed > 60 chars
	CHECK_WITH(strlen(target), _ret > 60);

	TEST_SUCC(symlink(target, link));

	char buf[PATH_MAX];
	TEST_RES(readlink(link, buf, sizeof(buf)),
		 _ret == (ssize_t)strlen(target));
	buf[strlen(target)] = '\0';
	TEST_RES(strcmp(buf, target), _ret == 0);

	unlink_if_exists(link);
	unlink_if_exists(target);
	for (int i = (int)(sizeof(dirs) / sizeof(dirs[0])) - 1; i >= 0; i--)
		rmdir_if_exists(dirs[i]);
}
END_TEST()

FN_TEST(symlink_too_long_enametoolong)
{
	const char *link = BASE_DIR "/toolong_link";

	// Build a target string that exceeds PATH_MAX (4096)
	char long_target[PATH_MAX + 2];
	memset(long_target, 'x', sizeof(long_target) - 1);
	long_target[sizeof(long_target) - 1] = '\0';

	TEST_ERRNO(symlink(long_target, link), ENAMETOOLONG);

	unlink_if_exists(link);
}
END_TEST()

FN_TEST(readlink_regular_file_einval)
{
	const char *file = BASE_DIR "/regular_file";

	int fd = TEST_SUCC(open(file, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	char buf[PATH_MAX];
	TEST_ERRNO(readlink(file, buf, sizeof(buf)), EINVAL);

	unlink_if_exists(file);
}
END_TEST()

FN_TEST(dangling_symlink_enoent)
{
	const char *link = BASE_DIR "/dangling_link";
	const char *target = BASE_DIR "/nonexistent_target";

	TEST_SUCC(symlink(target, link));

	// Opening through a dangling symlink should fail with ENOENT
	TEST_ERRNO(open(link, O_RDONLY), ENOENT);

	// The symlink itself should still exist
	struct stat st;
	TEST_SUCC(lstat(link, &st));

	unlink_if_exists(link);
}
END_TEST()
