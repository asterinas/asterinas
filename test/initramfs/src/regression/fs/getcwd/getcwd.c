// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(getcwd_small_buffer_returns_erange)
{
	char small[1];
	TEST_ERRNO(getcwd(small, 1), ERANGE);
}
END_TEST()

FN_SETUP(getcwd_long_path)
{
	CHECK_WITH(mkdir("/tmp/getcwd_long", 0755),
		   _ret == 0 || errno == EEXIST);
	CHECK(chdir("/tmp/getcwd_long"));

	char dirname[256];
	memset(dirname, 'x', 200);
	dirname[200] = '\0';

	for (int i = 0; i < 25; i++) {
		CHECK(mkdir(dirname, 0755));
		CHECK(chdir(dirname));
	}
}
END_SETUP()

FN_TEST(getcwd_enametoolong_when_path_exceeds_path_max)
{
	char buf[8192];
	TEST_ERRNO(syscall(SYS_getcwd, buf, sizeof(buf)), ENAMETOOLONG);
}
END_TEST()

FN_SETUP(getcwd_long_path_cleanup)
{
	char dirname[256];
	memset(dirname, 'x', 200);
	dirname[200] = '\0';

	for (int i = 0; i < 25; i++) {
		CHECK(chdir(".."));
		CHECK(rmdir(dirname));
	}
	CHECK(chdir("/"));
	CHECK(rmdir("/tmp/getcwd_long"));
}
END_SETUP()
