// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"

static void invalid_requests(void)
{
	struct sock_filter allow = BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW);
	struct sock_fprog prog = { .len = 1, .filter = &allow };
	CHECK_WITH(syscall(SYS_seccomp, -1, 0, NULL),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0x80000000,
			   NULL),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, NULL),
		   _ret == -1 && errno == EFAULT);
	CHECK_WITH(install(&allow, 0), _ret == -1 && errno == EINVAL);
	CHECK_WITH(install(&allow, 4097), _ret == -1 && errno == EINVAL);
	CHECK_WITH(prctl(PR_SET_SECCOMP, UINT64_C(0x100000002), &prog),
		   _ret == -1 && errno == EINVAL);
	no_new_privs();
	prog.filter = NULL;
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog),
		   _ret == -1 && errno == EINVAL);
	prog.filter = (void *)1;
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog),
		   _ret == -1 && errno == EFAULT);
	CHECK_WITH(prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, NULL),
		   _ret == -1 && errno == EFAULT);
}

static void two_installation_interfaces(void)
{
	struct sock_filter allow = BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW);
	struct sock_fprog prog = { .len = 1, .filter = &allow };
	no_new_privs();
	CHECK(install(&allow, 1));
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_FILTER);
	CHECK(prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &prog));
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_FILTER);
	CHECK(syscall(SYS_getpid));
}

static void unprivileged_installation(void)
{
	struct sock_filter allow = BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW);
	if (geteuid() == 0)
		CHECK(setuid(65534));
	CHECK_WITH(install(&allow, 1), _ret == -1 && errno == EACCES);
	/* Length validation precedes the authorization check. */
	CHECK_WITH(install(NULL, 0), _ret == -1 && errno == EINVAL);
	no_new_privs();
	CHECK(install(&allow, 1));
}

static void failed_installation_preserves_policy(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	struct sock_filter invalid = BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 64);
	CHECK_WITH(install(&invalid, 1), _ret == -1 && errno == EINVAL);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

FN_TEST(installation)
{
	TEST_RES(run_child(invalid_requests, 0), _ret == 0);
	TEST_RES(run_child(two_installation_interfaces, 0), _ret == 0);
	TEST_RES(run_child(unprivileged_installation, 0), _ret == 0);
	TEST_RES(run_child(failed_installation_preserves_policy, 0), _ret == 0);
}
END_TEST()

FN_TEST(notification_sizes)
{
	struct {
		struct seccomp_notif_sizes sizes;
		unsigned char guard[2];
	} result;
	memset(&result, 0xa5, sizeof(result));
	TEST_SUCC(syscall(SYS_seccomp, SECCOMP_GET_NOTIF_SIZES, 0,
			  &result.sizes));
	TEST_RES(result.sizes.seccomp_notif,
		 _ret == sizeof(struct seccomp_notif));
	TEST_RES(result.sizes.seccomp_notif_resp,
		 _ret == sizeof(struct seccomp_notif_resp));
	TEST_RES(result.sizes.seccomp_data,
		 _ret == sizeof(struct seccomp_data));
	TEST_RES(result.guard[0], _ret == 0xa5);
	TEST_RES(result.guard[1], _ret == 0xa5);
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_NOTIF_SIZES, 1, NULL),
		   EINVAL);
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_NOTIF_SIZES, 0, NULL),
		   EFAULT);
}
END_TEST()

FN_TEST(action_queries)
{
	uint32_t actions[] = { SECCOMP_RET_ALLOW, SECCOMP_RET_ERRNO,
			       SECCOMP_RET_KILL_PROCESS,
			       SECCOMP_RET_KILL_THREAD };
	for (size_t i = 0; i < sizeof(actions) / sizeof(actions[0]); ++i)
		TEST_SUCC(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0,
				  &actions[i]));
	uint32_t unknown = SECCOMP_RET_ERRNO | 1;
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0, &unknown),
		   EOPNOTSUPP);
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 1, NULL),
		   EINVAL);
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0, NULL),
		   EFAULT);
}
END_TEST()
