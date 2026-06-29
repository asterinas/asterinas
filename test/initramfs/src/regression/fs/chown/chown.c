// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_FILE "/tmp/chown_test"

FN_SETUP(create_test_file)
{
	int fd = CHECK(open(TEST_FILE, O_CREAT | O_WRONLY | O_TRUNC, 0644));
	CHECK(close(fd));
}
END_SETUP()

FN_TEST(chown_negative_uid_as_unsigned)
{
	// -2 cast to `uid_t` (32-bit unsigned) is 4294967294.
	// Linux treats all non-(-1) values as unsigned.
	TEST_SUCC(chown(TEST_FILE, -2, -1));

	struct stat st;
	TEST_RES(stat(TEST_FILE, &st), st.st_uid == (uid_t)-2);

	TEST_SUCC(chown(TEST_FILE, 0, 0));
}
END_TEST()

FN_TEST(chown_negative_gid_as_unsigned)
{
	TEST_SUCC(chown(TEST_FILE, -1, -2));

	struct stat st;
	TEST_RES(stat(TEST_FILE, &st), st.st_gid == (gid_t)-2);

	TEST_SUCC(chown(TEST_FILE, 0, 0));
}
END_TEST()

FN_TEST(chown_minus_one_no_change)
{
	TEST_SUCC(chown(TEST_FILE, 1000, 1000));

	// -1 means "no change" for both UID and GID.
	TEST_SUCC(chown(TEST_FILE, -1, -1));

	struct stat st;
	TEST_RES(stat(TEST_FILE, &st), st.st_uid == 1000 && st.st_gid == 1000);

	TEST_SUCC(chown(TEST_FILE, 0, 0));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(unlink(TEST_FILE));
}
END_SETUP()
