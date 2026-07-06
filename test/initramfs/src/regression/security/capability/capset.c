// SPDX-License-Identifier: MPL-2.0

#include <stdint.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <linux/capability.h>

#include "../../common/test.h"

static struct __user_cap_header_struct caphdr = {
	.version = _LINUX_CAPABILITY_VERSION_3
};
static struct __user_cap_data_struct capdat[2];

#define CAPS_ALL 0x000001ffffffffff
#define CAPS_NONE 0x0000000000000000

FN_TEST(capget)
{
	TEST_SUCC(syscall(SYS_capget, &caphdr, capdat));

	TEST_RES(capdat[0].effective | (((uint64_t)capdat[1].effective) << 32),
		 _ret == CAPS_ALL);
	TEST_RES(capdat[0].permitted | (((uint64_t)capdat[1].permitted) << 32),
		 _ret == CAPS_ALL);
	TEST_RES(capdat[0].inheritable |
			 (((uint64_t)capdat[1].inheritable) << 32),
		 _ret == CAPS_NONE);

	TEST_SUCC(syscall(SYS_capset, &caphdr, &capdat));
}
END_TEST()

FN_TEST(capset_inheritable_across_uid_transition)
{
	pid_t pid;
	int status;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		struct __user_cap_data_struct child_capdat[2] = {};

		CHECK(syscall(SYS_capget, &caphdr, child_capdat));
		child_capdat[0].inheritable |= 1 << CAP_NET_BIND_SERVICE;
		CHECK(syscall(SYS_capset, &caphdr, child_capdat));

		CHECK(setuid(65534));
		CHECK(syscall(SYS_capget, &caphdr, child_capdat));

		CHECK_WITH(child_capdat[0].permitted, _ret == 0);
		CHECK_WITH(child_capdat[0].effective, _ret == 0);
		CHECK_WITH(child_capdat[0].inheritable &
				   (1 << CAP_NET_BIND_SERVICE),
			   _ret != 0);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_TEST(capset_permitted)
{
	// Effective capabilities must be permitted.
	capdat[0].permitted -= 1 << CAP_SYS_ADMIN;
	TEST_ERRNO(syscall(SYS_capset, &caphdr, &capdat), EPERM);

	capdat[0].effective -= 1 << CAP_SYS_ADMIN;
	TEST_SUCC(syscall(SYS_capset, &caphdr, &capdat));

	// Adding permitted capabilities is not allowed.
	capdat[0].permitted += 1 << CAP_SYS_ADMIN;
	TEST_ERRNO(syscall(SYS_capset, &caphdr, &capdat), EPERM);
	capdat[0].permitted -= 1 << CAP_SYS_ADMIN;
}
END_TEST()

FN_TEST(capset_inheritable)
{
	// With CAP_SETPCAP, new inheritable capabilities may not be permitted.
	capdat[0].inheritable += 1 << CAP_SYS_ADMIN;
	TEST_SUCC(syscall(SYS_capset, &caphdr, &capdat));

	capdat[0].effective -= 1 << CAP_SETPCAP;
	TEST_SUCC(syscall(SYS_capset, &caphdr, &capdat));

	// Without CAP_SETPCAP, old inheritable capabilities may not be permitted.
	TEST_SUCC(syscall(SYS_capset, &caphdr, &capdat));

	capdat[0].inheritable -= 1 << CAP_SYS_ADMIN;
	TEST_SUCC(syscall(SYS_capset, &caphdr, &capdat));

	// Without CAP_SETPCAP, new inheritable capabilities must be permitted.
	capdat[0].inheritable += 1 << CAP_SYS_ADMIN;
	TEST_ERRNO(syscall(SYS_capset, &caphdr, &capdat), EPERM);
	capdat[0].inheritable -= 1 << CAP_SYS_ADMIN;

	// Without CAP_SETPCAP, new inheritable capabilities may not be effective.
	capdat[0].inheritable += 1 << CAP_SETPCAP;
	TEST_SUCC(syscall(SYS_capset, &caphdr, &capdat));
}
END_TEST()
