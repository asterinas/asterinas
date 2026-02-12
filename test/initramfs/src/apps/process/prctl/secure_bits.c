// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <sys/prctl.h>
#include <sys/wait.h>
#include <linux/capability.h>
#include <linux/securebits.h>
#include <unistd.h>

FN_TEST(get_initial_securebits)
{
	// Initially, all securebits should be 0
	TEST_RES(prctl(PR_GET_SECUREBITS), _ret == 0);
}
END_TEST()

FN_TEST(set_and_get_keep_caps)
{
	int initial_bits;

	initial_bits = TEST_SUCC(prctl(PR_GET_SECUREBITS));

	// Set KEEP_CAPS bit
	TEST_SUCC(prctl(PR_SET_SECUREBITS, initial_bits | SECBIT_KEEP_CAPS));
	TEST_RES(prctl(PR_GET_SECUREBITS),
		 (_ret & SECBIT_KEEP_CAPS) == SECBIT_KEEP_CAPS);

	// Clear KEEP_CAPS bit
	TEST_SUCC(prctl(PR_SET_SECUREBITS, initial_bits & ~SECBIT_KEEP_CAPS));
	TEST_RES(prctl(PR_GET_SECUREBITS), (_ret & SECBIT_KEEP_CAPS) == 0);

	// Restore initial state
	TEST_SUCC(prctl(PR_SET_SECUREBITS, initial_bits));
}
END_TEST()

FN_TEST(set_multiple_bits)
{
	int initial_bits;
	int combined_bits;

	initial_bits = TEST_SUCC(prctl(PR_GET_SECUREBITS));
	combined_bits = SECBIT_KEEP_CAPS | SECBIT_NO_SETUID_FIXUP;

	// Set both KEEP_CAPS and NO_SETUID_FIXUP
	TEST_SUCC(prctl(PR_SET_SECUREBITS, initial_bits | combined_bits));
	TEST_RES(prctl(PR_GET_SECUREBITS),
		 (_ret & combined_bits) == combined_bits);

	// Restore initial state
	TEST_SUCC(prctl(PR_SET_SECUREBITS, initial_bits));
}
END_TEST()

FN_TEST(lock_keep_caps_bit)
{
	int initial_bits;

	initial_bits = TEST_SUCC(prctl(PR_GET_SECUREBITS));

	// Set and lock KEEP_CAPS bit
	TEST_SUCC(prctl(PR_SET_SECUREBITS, initial_bits | SECBIT_KEEP_CAPS |
						   SECBIT_KEEP_CAPS_LOCKED));
	TEST_RES(prctl(PR_GET_SECUREBITS),
		 (_ret & (SECBIT_KEEP_CAPS | SECBIT_KEEP_CAPS_LOCKED)) ==
			 (SECBIT_KEEP_CAPS | SECBIT_KEEP_CAPS_LOCKED));

	// Try to clear locked KEEP_CAPS bit - should fail
	TEST_ERRNO(prctl(PR_SET_SECUREBITS, initial_bits & ~SECBIT_KEEP_CAPS),
		   EPERM);

	// Try to unlock KEEP_CAPS_LOCKED bit - should fail
	TEST_ERRNO(prctl(PR_SET_SECUREBITS,
			 initial_bits & ~SECBIT_KEEP_CAPS_LOCKED),
		   EPERM);

	// Verify bit is still set and locked
	TEST_RES(prctl(PR_GET_SECUREBITS),
		 (_ret & (SECBIT_KEEP_CAPS | SECBIT_KEEP_CAPS_LOCKED)) ==
			 (SECBIT_KEEP_CAPS | SECBIT_KEEP_CAPS_LOCKED));
}
END_TEST()