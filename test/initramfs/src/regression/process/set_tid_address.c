// SPDX-License-Identifier: MPL-2.0

#include "../common/test.h"

#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

// Regression test for #3962.
FN_TEST(replaces_existing_clear_child_tid)
{
	int tidptr = 0;
	TEST_RES(syscall(SYS_set_tid_address, &tidptr), _ret > 0);

	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		long tid = syscall(SYS_set_tid_address, &tidptr);
		_exit(tid > 0 ? EXIT_SUCCESS : EXIT_FAILURE);
	}

	int status;
	TEST_RES(waitpid(child, &status, 0), _ret == child);
	TEST_RES(status,
		 WIFEXITED(status) && WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()
