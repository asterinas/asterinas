// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/ext2/symlink_test"
#define TARGET_DIR TEST_DIR "/dir"
#define SYMLINK TEST_DIR "/lnk"

FN_SETUP(prepare)
{
	CHECK(mkdir(TEST_DIR, 0755));
	CHECK(mkdir(TARGET_DIR, 0755));
	CHECK(symlink("dir", SYMLINK));
}
END_SETUP()

FN_TEST(symlink_trailing_slash_nofollow)
{
	// lnk -> dir, O_NOFOLLOW + trailing slash should succeed
	// (trailing slash implies directory, symlink must be followed)
	int fd = TEST_SUCC(open(SYMLINK "/", O_RDONLY | O_NOFOLLOW));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(open_creat_excl_nofollow_symlink)
{
	// O_CREAT | O_EXCL | O_NOFOLLOW on an existing symlink should return EEXIST
	TEST_ERRNO(open(SYMLINK, O_CREAT | O_EXCL | O_NOFOLLOW | O_RDWR, 0644),
		   EEXIST);
}
END_TEST()

FN_SETUP(cleanup)
{
	unlink(SYMLINK);
	rmdir(TARGET_DIR);
	rmdir(TEST_DIR);
}
END_SETUP()
