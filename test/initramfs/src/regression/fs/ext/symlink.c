// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"
#include "fs_test.h"

#define BASE_DIR EXT_TEST_ROOT "/shared_symlink_test"

FN_SETUP(prepare_base_dir)
{
	CHECK(mkdir(BASE_DIR, 0755));
}
END_SETUP()

FN_TEST(symlink_short_roundtrip)
{
	const char *file = BASE_DIR "/short_file";
	const char *link = BASE_DIR "/short_link";
	const char *data = "hello symlink";

	int fd = TEST_SUCC(open(file, O_CREAT | O_WRONLY, 0644));
	TEST_RES(write(fd, data, strlen(data)), _ret == strlen(data));
	TEST_SUCC(close(fd));

	TEST_SUCC(symlink(file, link));

	char buf[PATH_MAX];
	TEST_RES(readlink(link, buf, sizeof(buf)),
		 _ret == strlen(file) && memcmp(buf, file, strlen(file)) == 0);

	// Open via symlink and verify content
	char rbuf[64];
	fd = TEST_SUCC(open(link, O_RDONLY));
	TEST_RES(read(fd, rbuf, sizeof(rbuf)),
		 _ret == strlen(data) && memcmp(rbuf, data, strlen(data)) == 0);
	TEST_SUCC(close(fd));

	TEST_SUCC(unlink(link));
	TEST_SUCC(unlink(file));
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
		TEST_SUCC(mkdir(dirs[i], 0755));

	int fd = TEST_SUCC(open(target, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	// Verify the target path is indeed > 60 chars
	TEST_RES(strlen(target), _ret > 60);

	TEST_SUCC(symlink(target, link));

	char buf[PATH_MAX];
	TEST_RES(readlink(link, buf, sizeof(buf)),
		 _ret == strlen(target) &&
			 memcmp(buf, target, strlen(target)) == 0);

	TEST_SUCC(unlink(link));
	TEST_SUCC(unlink(target));
	for (int i = (int)(sizeof(dirs) / sizeof(dirs[0])) - 1; i >= 0; i--)
		TEST_SUCC(rmdir(dirs[i]));
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
}
END_TEST()

FN_TEST(readlink_regular_file_einval)
{
	const char *file = BASE_DIR "/regular_file";

	int fd = TEST_SUCC(open(file, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	char buf[PATH_MAX];
	TEST_ERRNO(readlink(file, buf, sizeof(buf)), EINVAL);

	TEST_SUCC(unlink(file));
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

	TEST_SUCC(unlink(link));
}
END_TEST()

FN_SETUP(cleanup_base_dir)
{
	CHECK(rmdir(BASE_DIR));
}
END_SETUP()
