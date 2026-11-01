/* SPDX-License-Identifier: MPL-2.0 */

/*
 * A read that reaches EOF must leave bytes beyond the returned count unchanged.
 */

#include <fcntl.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(pread_across_eof_leaves_tail_untouched)
{
	char buf[64];
	char expected[sizeof(buf)];
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR | O_CREAT | O_TRUNC, 0600));

	TEST_RES(write(fd, "hi", 2), _ret == 2);
	memset(buf, 'x', sizeof(buf));
	memset(expected, 'x', sizeof(expected));
	expected[0] = 'i';
	TEST_RES(pread(fd, buf, sizeof(buf), 1),
		 _ret == 1 && memcmp(buf, expected, sizeof(buf)) == 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE));
}
END_TEST()
