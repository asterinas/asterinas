// SPDX-License-Identifier: MPL-2.0
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>
#include "../../common/test.h"

#define TEST_DIR "/ext2/open_dir_test"
#define TARGET_DIR TEST_DIR "/objd"

FN_SETUP(prepare)
{
	CHECK(mkdir(TEST_DIR, 0755));
	CHECK(mkdir(TARGET_DIR, 0755));
}
END_SETUP()

FN_TEST(open_creat_directory_on_existing_dir_returns_einval)
{
	/* Fix A: O_CREAT | O_DIRECTORY on an existing directory should return EINVAL */
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_DIRECTORY | O_RDONLY, 0644),
		   EINVAL);
}
END_TEST()

FN_TEST(open_creat_on_existing_dir_returns_eisdir)
{
	/* Fix B: O_CREAT on an existing directory should return EISDIR */
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_WRONLY, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_creat_excl_on_existing_dir_returns_eisdir)
{
	/* Fix B: O_CREAT | O_EXCL on an existing directory should return EISDIR */
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_EXCL | O_WRONLY, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_wronly_on_dir_returns_eisdir)
{
	/* Fix C: O_WRONLY on a directory should return EISDIR */
	TEST_ERRNO(open(TARGET_DIR, O_WRONLY), EISDIR);
}
END_TEST()

FN_TEST(open_rdwr_on_dir_returns_eisdir)
{
	/* Fix C: O_RDWR on a directory should return EISDIR */
	TEST_ERRNO(open(TARGET_DIR, O_RDWR), EISDIR);
}
END_TEST()

FN_SETUP(cleanup)
{
	rmdir(TARGET_DIR);
	rmdir(TEST_DIR);
}
END_SETUP()
