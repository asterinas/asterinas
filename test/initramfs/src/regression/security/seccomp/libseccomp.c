// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include <seccomp.h>

static void generated_filter(void)
{
	no_new_privs();
	scmp_filter_ctx ctx =
		CHECK_WITH(seccomp_init(SCMP_ACT_ALLOW), _ret != NULL);
	CHECK(seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EBUSY), SCMP_SYS(getppid),
			       0));
	CHECK(seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EACCES), SCMP_SYS(close), 1,
			       SCMP_A0(SCMP_CMP_EQ,
				       UINT64_C(0x1234567889abcdef))));
	CHECK(seccomp_load(ctx));
	seccomp_release(ctx);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	CHECK_WITH(syscall(SYS_close, UINT64_C(0x1234567889abcdef)),
		   _ret == -1 && errno == EACCES);
	CHECK_WITH(syscall(SYS_close, UINT64_C(0x1234567889abcdee)),
		   _ret == -1 && errno == EBADF);
}

FN_TEST(libseccomp_filter)
{
	TEST_RES(run_child(generated_filter, 0), _ret == 0);
}
END_TEST()
