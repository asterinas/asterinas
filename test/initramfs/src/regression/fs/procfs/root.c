// SPDX-License-Identifier: MPL-2.0

#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(proc_self_root_is_root)
{
	char buf[64];
	struct stat st;
	ssize_t len;

	TEST_SUCC(lstat("/proc/self/root", &st));
	TEST_RES(S_ISLNK(st.st_mode), _ret != 0);

	len = TEST_RES(readlink("/proc/self/root", buf, sizeof(buf) - 1),
		       _ret > 0 && _ret < (ssize_t)sizeof(buf));
	buf[len] = '\0';
	TEST_RES(strcmp(buf, "/"), _ret == 0);
}
END_TEST()
