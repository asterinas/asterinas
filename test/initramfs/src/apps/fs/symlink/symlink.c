// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/ext2/symlink_test"
#define DIRECTORY_TARGET TEST_DIR "/dir"
#define REGULAR_TARGET TEST_DIR "/file"
#define DIRECTORY_SYMLINK TEST_DIR "/dir_link"
#define REGULAR_SYMLINK TEST_DIR "/file_link"

FN_SETUP(prepare)
{
	int fd = -1;

	CHECK(mkdir(TEST_DIR, 0755));
	CHECK(mkdir(DIRECTORY_TARGET, 0755));

	fd = CHECK(open(REGULAR_TARGET, O_CREAT | O_RDWR | O_TRUNC, 0644));
	CHECK(close(fd));

	CHECK(symlink("dir", DIRECTORY_SYMLINK));
	CHECK(symlink("file", REGULAR_SYMLINK));
}
END_SETUP()

FN_TEST(symlink_trailing_slash_nofollow)
{
	// `dir_link` points to a directory, so the trailing slash forces directory
	// semantics and the symlink must still be resolved with `O_NOFOLLOW`.
	int fd = TEST_SUCC(open(DIRECTORY_SYMLINK "/", O_RDONLY | O_NOFOLLOW));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(symlink_trailing_slash_nofollow_non_directory_returns_enotdir)
{
	// `file_link` points to a regular file, so the trailing slash still forces
	// directory semantics but the post-resolution check must reject it.
	TEST_ERRNO(open(REGULAR_SYMLINK "/", O_RDONLY | O_NOFOLLOW), ENOTDIR);
}
END_TEST()

FN_TEST(open_creat_excl_nofollow_symlink)
{
	// `O_CREAT | O_EXCL | O_NOFOLLOW` on an existing symlink should return
	// `EEXIST` before the symlink-specific `ELOOP`.
	TEST_ERRNO(open(DIRECTORY_SYMLINK,
			O_CREAT | O_EXCL | O_NOFOLLOW | O_RDWR, 0644),
		   EEXIST);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(unlink(REGULAR_SYMLINK));
	CHECK(unlink(DIRECTORY_SYMLINK));
	CHECK(unlink(REGULAR_TARGET));
	CHECK(rmdir(DIRECTORY_TARGET));
	CHECK(rmdir(TEST_DIR));
}
END_SETUP()
