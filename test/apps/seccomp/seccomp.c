// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../network/test.h"

#include <linux/seccomp.h>
#include <sys/syscall.h>
#include <unistd.h>

FN_SETUP()
{
}
END_SETUP()

FN_TEST(set_mode_strict_with_invalid_args)
{
	// Test with non-zero flags
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_SET_MODE_STRICT, 1, NULL),
		   EINVAL);

	// Test with non-zero uargs
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_SET_MODE_STRICT, 0, (void *)1),
		   EINVAL);
}
END_TEST()

FN_TEST(get_notif_sizes_with_bad_flags)
{
	struct seccomp_notif_sizes notif_sizes;

	// Test with non-zero flags
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_NOTIF_SIZES, 1,
			   &notif_sizes),
		   EINVAL);

	// Test with zero flags (valid case)
	TEST_SUCC(
		syscall(SYS_seccomp, SECCOMP_GET_NOTIF_SIZES, 0, &notif_sizes));
}
END_TEST()

FN_TEST(get_notif_sizes_with_null_ptr)
{
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_NOTIF_SIZES, 0, NULL),
		   EFAULT);
}
END_TEST()

FN_TEST(get_action_avail_with_bad_flags)
{
	// Test with non-zero flags
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 1,
			   SECCOMP_RET_KILL_PROCESS),
		   EINVAL);
}
END_TEST()

FN_TEST(get_action_avail_with_valid_actions)
{
	// All these tests should pass with zero flags
	unsigned int action[8] = { 0 };
	action[0] = SECCOMP_RET_KILL_PROCESS;
	action[1] = SECCOMP_RET_KILL_THREAD;
	action[2] = SECCOMP_RET_TRAP;
	action[3] = SECCOMP_RET_ERRNO;
	action[4] = SECCOMP_RET_USER_NOTIF;
	action[5] = SECCOMP_RET_TRACE;
	action[6] = SECCOMP_RET_LOG;
	action[7] = SECCOMP_RET_ALLOW;

	for (int i = 0; i < 8; i++) {
		TEST_SUCC(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0,
				  &action[i]));
	}
}
END_TEST()

FN_TEST(get_action_avail_with_invalid_action)
{
	unsigned int action = 0x12345678;
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0, &action),
		   EOPNOTSUPP);
}
END_TEST()

FN_TEST(get_action_avail_with_valid_action)
{
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0,
			   0x12345678),
		   EFAULT);
}
END_TEST()

FN_TEST(invalid_operation)
{
	TEST_ERRNO(syscall(SYS_seccomp, 0xffffffff, 0, NULL), EINVAL);
}
END_TEST()
