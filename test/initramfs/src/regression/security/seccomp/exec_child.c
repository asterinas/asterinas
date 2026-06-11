// SPDX-License-Identifier: MPL-2.0

// Helper executed by `exec.c` to verify that a seccomp filter and the
// `no_new_privs` bit installed before `execve` are still in force afterwards.

#define _GNU_SOURCE

#include "../../common/test.h"
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <unistd.h>

FN_TEST(seccomp_is_inherited_across_execve)
{
	// `no_new_privs` survives `execve`.
	TEST_RES(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 1);

	// The filter installed before `execve` still denies `getppid`.
	TEST_ERRNO(syscall(SYS_getppid), EPERM);
}
END_TEST()
