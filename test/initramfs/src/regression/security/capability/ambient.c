// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>
#include <linux/capability.h>

#include "../../common/capability.h"
#include "../../common/test.h"

static int update_inheritable(int cap, int add)
{
	struct __user_cap_data_struct cap_data[2] = {};
	unsigned int cap_index = cap / 32;
	uint32_t cap_mask = 1U << (cap % 32);

	if (__read_cap_data(cap_data) < 0)
		return -1;

	if (add)
		cap_data[cap_index].inheritable |= cap_mask;
	else
		cap_data[cap_index].inheritable &= ~cap_mask;

	return __write_cap_data(cap_data);
}

static int add_inheritable(int cap)
{
	return update_inheritable(cap, 1);
}

static int remove_inheritable(int cap)
{
	return update_inheritable(cap, 0);
}

static int clear_ambient(void)
{
	return prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0);
}

static int reset_capability_state(void)
{
	static const int caps[] = {
		CAP_SYS_ADMIN,
		CAP_NET_BIND_SERVICE,
		CAP_NET_RAW,
		CAP_WAKE_ALARM,
	};
	size_t i;

	if (clear_ambient() < 0)
		return -1;

	for (i = 0; i < sizeof(caps) / sizeof(caps[0]); i++) {
		if (remove_inheritable(caps[i]) < 0)
			return -1;
	}

	return 0;
}

FN_TEST(ambient_initially_empty)
{
	TEST_SUCC(reset_capability_state());

	// All capabilities should not be in the ambient set after reset.
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, CAP_SYS_ADMIN, 0,
		       0),
		 _ret == 0);
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
		       CAP_NET_BIND_SERVICE, 0, 0),
		 _ret == 0);
}
END_TEST()

FN_TEST(ambient_raise_and_query)
{
	TEST_SUCC(reset_capability_state());

	// CAP_SYS_ADMIN must be both permitted and inheritable to raise.
	// As root, the permitted set already has it. Make it inheritable.
	TEST_SUCC(add_inheritable(CAP_SYS_ADMIN));

	// Raise the capability.
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, CAP_SYS_ADMIN, 0,
			0));
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, CAP_SYS_ADMIN, 0,
		       0),
		 _ret == 1);

	// Another capability should still not be set.
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
		       CAP_NET_BIND_SERVICE, 0, 0),
		 _ret == 0);
}
END_TEST()

FN_TEST(ambient_lower)
{
	TEST_SUCC(reset_capability_state());

	// Raise the capability first.
	TEST_SUCC(add_inheritable(CAP_SYS_ADMIN));
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, CAP_SYS_ADMIN, 0,
			0));

	// Lower the capability.
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_LOWER, CAP_SYS_ADMIN, 0,
			0));
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, CAP_SYS_ADMIN, 0,
		       0),
		 _ret == 0);

	// Lowering an already-absent capability should succeed.
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_LOWER, CAP_SYS_ADMIN, 0,
			0));
}
END_TEST()

FN_TEST(ambient_clear_all)
{
	TEST_SUCC(reset_capability_state());

	// Raise a few capabilities first.
	TEST_SUCC(add_inheritable(CAP_SYS_ADMIN));
	TEST_SUCC(add_inheritable(CAP_NET_BIND_SERVICE));
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, CAP_SYS_ADMIN, 0,
			0));
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE,
			CAP_NET_BIND_SERVICE, 0, 0));
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, CAP_SYS_ADMIN, 0,
		       0),
		 _ret == 1);
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
		       CAP_NET_BIND_SERVICE, 0, 0),
		 _ret == 1);

	// Clear all.
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0));

	// Everything should be gone.
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, CAP_SYS_ADMIN, 0,
		       0),
		 _ret == 0);
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
		       CAP_NET_BIND_SERVICE, 0, 0),
		 _ret == 0);
}
END_TEST()

FN_TEST(ambient_raise_requires_inheritable)
{
	TEST_SUCC(reset_capability_state());

	// Remove CAP_NET_RAW from the inheritable set, then try to raise it.
	TEST_SUCC(remove_inheritable(CAP_NET_RAW));
	TEST_ERRNO(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, CAP_NET_RAW, 0,
			 0),
		   EPERM);
}
END_TEST()

FN_TEST(ambient_raise_ignores_bounding)
{
	pid_t pid;
	int status;

	TEST_SUCC(reset_capability_state());
	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(add_inheritable(CAP_WAKE_ALARM));
		CHECK(prctl(PR_CAPBSET_DROP, CAP_WAKE_ALARM, 0, 0, 0));

		CHECK(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE,
			    CAP_WAKE_ALARM, 0, 0));
		CHECK_WITH(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
				 CAP_WAKE_ALARM, 0, 0),
			   _ret == 1);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(ambient_cleared_on_inheritable_drop)
{
	TEST_SUCC(reset_capability_state());

	// Raise CAP_NET_BIND_SERVICE into the ambient set.
	TEST_SUCC(add_inheritable(CAP_NET_BIND_SERVICE));
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE,
			CAP_NET_BIND_SERVICE, 0, 0));
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
		       CAP_NET_BIND_SERVICE, 0, 0),
		 _ret == 1);

	// Remove it from the inheritable set. It should automatically
	// vanish from the ambient set.
	TEST_SUCC(remove_inheritable(CAP_NET_BIND_SERVICE));
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
		       CAP_NET_BIND_SERVICE, 0, 0),
		 _ret == 0);
}
END_TEST()

FN_TEST(ambient_rejects_invalid_args)
{
	TEST_SUCC(reset_capability_state());

	TEST_ERRNO(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
			 CAP_LAST_CAP + 1, 0, 0),
		   EINVAL);
	TEST_ERRNO(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, CAP_SYS_ADMIN,
			 1, 0),
		   EINVAL);
	TEST_ERRNO(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, CAP_SYS_ADMIN, 0,
			 1),
		   EINVAL);
	TEST_ERRNO(prctl(PR_CAP_AMBIENT, 0, CAP_SYS_ADMIN, 0, 0), EINVAL);
	TEST_ERRNO(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL,
			 CAP_SYS_ADMIN, 0, 0),
		   EINVAL);
	TEST_ERRNO(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 1, 0),
		   EINVAL);
}
END_TEST()

FN_TEST(ambient_inherited_across_fork)
{
	pid_t pid;
	int status;

	TEST_SUCC(reset_capability_state());
	TEST_SUCC(add_inheritable(CAP_NET_BIND_SERVICE));
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE,
			CAP_NET_BIND_SERVICE, 0, 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		// Child should inherit parent's ambient set.
		CHECK_WITH(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
				 CAP_NET_BIND_SERVICE, 0, 0),
			   _ret == 1);

		// Child clears its ambient set; parent should be unaffected.
		CHECK(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0));
		CHECK_WITH(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
				 CAP_NET_BIND_SERVICE, 0, 0),
			   _ret == 0);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	// Parent's ambient set should still contain the capability.
	TEST_RES(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
		       CAP_NET_BIND_SERVICE, 0, 0),
		 _ret == 1);

	// Clean up.
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_LOWER,
			CAP_NET_BIND_SERVICE, 0, 0));
	TEST_SUCC(remove_inheritable(CAP_NET_BIND_SERVICE));
}
END_TEST()

FN_TEST(ambient_cleared_on_uid_transition)
{
	pid_t pid;
	int status;

	TEST_SUCC(reset_capability_state());
	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(add_inheritable(CAP_NET_BIND_SERVICE));
		CHECK(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE,
			    CAP_NET_BIND_SERVICE, 0, 0));

		// The ambient set must be cleared on the user transition
		// (root -> nobody).
		CHECK(setuid(65534));
		CHECK_WITH(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
				 CAP_NET_BIND_SERVICE, 0, 0),
			   _ret == 0);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(ambient_not_protected_by_keep_caps)
{
	pid_t pid;
	int status;

	TEST_SUCC(reset_capability_state());
	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(add_inheritable(CAP_NET_BIND_SERVICE));
		CHECK(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE,
			    CAP_NET_BIND_SERVICE, 0, 0));

		// Enable KEEPCAPS. The ambient set should still be cleared
		// on the user transition (root -> nobody) because KEEPCAPS
		// does not protect ambient capabilities.
		CHECK(prctl(PR_SET_KEEPCAPS, 1));
		CHECK(setuid(65534));
		CHECK_WITH(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET,
				 CAP_NET_BIND_SERVICE, 0, 0),
			   _ret == 0);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(ambient_raise_requires_permitted)
{
	pid_t pid;
	int status;

	TEST_SUCC(reset_capability_state());
	pid = TEST_SUCC(fork());
	if (pid == 0) {
		struct __user_cap_data_struct cap_data[2] = {};

		CHECK(add_inheritable(CAP_NET_RAW));

		// Drop CAP_NET_RAW from effective and permitted sets.
		read_cap_data(cap_data);
		cap_data[0].permitted &= ~(1U << CAP_NET_RAW);
		cap_data[0].effective &= ~(1U << CAP_NET_RAW);
		write_cap_data(cap_data);

		// PR_CAP_AMBIENT_RAISE should fail because the capability
		// is not in the permitted set.
		CHECK_WITH(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE,
				 CAP_NET_RAW, 0, 0),
			   _ret == -1 && errno == EPERM);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

static uint64_t read_proc_capamb(void)
{
	FILE *fp;
	char line[256];
	unsigned long long value;
	uint64_t capamb = 0;

	fp = CHECK_WITH(fopen("/proc/self/status", "r"), _ret != NULL);
	while (fgets(line, sizeof(line), fp) != NULL) {
		if (sscanf(line, "CapAmb:\t%llx", &value) == 1) {
			capamb = (uint64_t)value;
			break;
		}
	}
	CHECK(fclose(fp));
	return capamb;
}

FN_TEST(ambient_procfs_reflects_state)
{
	TEST_SUCC(reset_capability_state());

	TEST_SUCC(add_inheritable(CAP_NET_BIND_SERVICE));

	// Raise a capability and verify /proc/self/status reflects it.
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE,
			CAP_NET_BIND_SERVICE, 0, 0));
	TEST_RES(read_proc_capamb(),
		 (_ret & (1ULL << CAP_NET_BIND_SERVICE)) != 0);

	// Clear and verify it is gone from procfs.
	TEST_SUCC(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0));
	TEST_RES(read_proc_capamb(), _ret == 0);

	TEST_SUCC(remove_inheritable(CAP_NET_BIND_SERVICE));
}
END_TEST()
