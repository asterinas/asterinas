// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <sys/prctl.h>
#include <linux/capability.h>

#include "../../common/test.h"

#ifndef PR_CAP_AMBIENT
#define PR_CAP_AMBIENT 47
#endif

#ifndef PR_CAP_AMBIENT_IS_SET
#define PR_CAP_AMBIENT_IS_SET 1
#endif

#ifndef PR_CAP_AMBIENT_RAISE
#define PR_CAP_AMBIENT_RAISE 2
#endif

#ifndef PR_CAP_AMBIENT_LOWER
#define PR_CAP_AMBIENT_LOWER 3
#endif

#ifndef PR_CAP_AMBIENT_CLEAR_ALL
#define PR_CAP_AMBIENT_CLEAR_ALL 4
#endif

FN_TEST(ambient_empty_set)
{
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0));
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, CAP_SYS_ADMIN, 0,
		       0),
		 _ret == 0);
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_LOWER, CAP_SYS_ADMIN, 0,
			0));
}
END_TEST()

FN_TEST(ambient_unsupported_raise)
{
	TEST_ERRNO(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, CAP_SYS_ADMIN, 0,
			 0),
		   EPERM);
}
END_TEST()

FN_TEST(ambient_rejects_invalid_args)
{
	TEST_ERRNO(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
			 CAP_LAST_CAP + 1, 0, 0),
		   EINVAL);
	TEST_ERRNO(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL,
			 CAP_SYS_ADMIN, 0, 0),
		   EINVAL);
	TEST_ERRNO(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 1, 0),
		   EINVAL);
}
END_TEST()
