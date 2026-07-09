// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <linux/prctl.h>
#include <sys/auxv.h>
#include <sys/prctl.h>
#include <unistd.h>

#define NOBODY_UID 65534

FN_TEST(check_exec_state)
{
	int no_new_privs;
	uid_t uid;

	no_new_privs = TEST_SUCC(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0));

	if (no_new_privs) {
		uid = TEST_SUCC(getuid());
		TEST_RES(geteuid(), _ret == uid);
		TEST_RES(getauxval(AT_SECURE), _ret == 0);
		return;
	}

	TEST_RES(getuid(), _ret == NOBODY_UID);
	TEST_RES(geteuid(), _ret == 0);
	TEST_RES(getauxval(AT_SECURE), _ret == 1);
}
END_TEST()
