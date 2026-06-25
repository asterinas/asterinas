// SPDX-License-Identifier: MPL-2.0

#include <limits.h>
#include <linux/magic.h>
#include <sys/vfs.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(proc_statfs)
{
	struct statfs st;

	TEST_SUCC(statfs("/proc", &st));
	TEST_RES(st.f_type, _ret == PROC_SUPER_MAGIC);
	TEST_RES(st.f_bsize, _ret == getpagesize());
	TEST_RES(st.f_namelen, _ret == NAME_MAX);
}
END_TEST()
