// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../network/test.h"
#include <sys/prctl.h>
#include <linux/capability.h>
#include <string.h>

FN_SETUP()
{
}

END_SETUP()

FN_TEST(test_prctl_capbset_read)
{
	int ret;
	unsigned long cap;

	for (cap = 0; cap <= 40; cap++) {
		ret = prctl(PR_CAPBSET_READ, cap);
		TEST_SUCC(ret);
	}
}

END_TEST()

FN_TEST(test_prctl_cap_ambient)
{
	int ret;

	ret = prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, 40, 0, 0);
	TEST_SUCC(ret);
}

END_TEST()

FN_TEST(test_prctl_no_new_privs)
{
	int ret;

	ret = prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0);
	TEST_SUCC(ret);
}

END_TEST()

FN_TEST(test_prctl_securebits)
{
	int ret;

	ret = prctl(PR_GET_SECUREBITS, 0, 0, 0, 0);
	TEST_SUCC(ret);
}

END_TEST()

FN_TEST(test_prctl_dumpable)
{
	int ret;

	ret = prctl(PR_GET_DUMPABLE);
	TEST_RES(ret, ret == 0);
}

END_TEST()