// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <unistd.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <linux/capability.h>

#include "../../common/test.h"

static uid_t root = 0;
static uid_t nobody = 65534;

#define CAPS_ALL "000001ffffffffff"
#define CAPS_NONE "0000000000000000"

static char child_path[4096];

static int clear_caps(void)
{
	struct __user_cap_header_struct hdr;
	struct __user_cap_data_struct data[2];

	hdr.version = _LINUX_CAPABILITY_VERSION_3;
	hdr.pid = 0;
	memset(data, 0, sizeof(data));

	return syscall(SYS_capset, &hdr, data);
}

static int noop(void)
{
	return 0;
}

FN_SETUP(child_path)
{
	CHECK(readlink("/proc/self/exe", child_path, sizeof(child_path) - 10));
	strcat(child_path, "_child");
}
END_SETUP()

#define TEST_CAPS_AFTER_EXECVE(name, ruid, euid, suid, func, ecaps, pcaps,  \
			       icaps)                                       \
	FN_TEST(name)                                                       \
	{                                                                   \
		pid_t pid;                                                  \
		int status;                                                 \
                                                                            \
		pid = TEST_SUCC(fork());                                    \
		if (pid == 0) {                                             \
			CHECK(setresuid(ruid, euid, suid));                 \
			CHECK(func());                                      \
			CHECK(execl(child_path, child_path, ecaps, pcaps,   \
				    icaps, NULL));                          \
		}                                                           \
                                                                            \
		TEST_RES(wait(&status), _ret == pid && WIFEXITED(status) && \
						WEXITSTATUS(status) == 0);  \
	}                                                                   \
	END_TEST()

// ===========================================================
// Tests whose initial state does not contain any capabilities
// ===========================================================

#define TEST_EXECVE_GAIN_CAPS(name, ruid, euid, suid)                        \
	TEST_CAPS_AFTER_EXECVE(name, ruid, euid, suid, clear_caps, CAPS_ALL, \
			       CAPS_ALL, CAPS_NONE)

#define TEST_EXECVE_NO_GAIN_CAPS(name, ruid, euid, suid, pcaps)               \
	TEST_CAPS_AFTER_EXECVE(name, ruid, euid, suid, clear_caps, CAPS_NONE, \
			       pcaps, CAPS_NONE)

// Effective UID = 0
//
// Final State:
// Effective capabilities = CAPS_ALL, permitted capabilities = CAPS_ALL
TEST_EXECVE_GAIN_CAPS(rrr_gain_caps, root, root, root);
TEST_EXECVE_GAIN_CAPS(rrn_gain_caps, root, root, nobody);
TEST_EXECVE_GAIN_CAPS(nrr_gain_caps, nobody, root, root);
TEST_EXECVE_GAIN_CAPS(nrn_gain_caps, nobody, root, nobody);

// Effective UID != 0, Real UID = 0
//
// Final State:
// Effective capabilities = CAPS_NONE, permitted capabilities = CAPS_ALL
TEST_EXECVE_NO_GAIN_CAPS(rnr_no_gain_caps, root, nobody, root, CAPS_ALL);
TEST_EXECVE_NO_GAIN_CAPS(rnn_no_gain_caps, root, nobody, nobody, CAPS_ALL);

// Effective UID != 0, Real UID != 0
//
// Final State:
// Effective capabilities = CAPS_NONE, permitted capabilities = CAPS_NONE
TEST_EXECVE_NO_GAIN_CAPS(nnr_no_gain_caps, nobody, nobody, root, CAPS_NONE);
TEST_EXECVE_NO_GAIN_CAPS(nnn_no_gain_caps, nobody, nobody, nobody, CAPS_NONE);

// ===================================================
// Tests whose initial state contains all capabilities
// ===================================================

#define TEST_EXECVE_NO_LOST_CAPS(name, ruid, euid, suid)               \
	TEST_CAPS_AFTER_EXECVE(name, ruid, euid, suid, noop, CAPS_ALL, \
			       CAPS_ALL, CAPS_NONE)

#define TEST_EXECVE_LOST_CAPS(name, ruid, euid, suid, pcaps)                   \
	TEST_CAPS_AFTER_EXECVE(name, ruid, euid, suid, noop, CAPS_NONE, pcaps, \
			       CAPS_NONE)

// Effective UID = 0
//
// Final State:
// Effective capabilities = CAPS_ALL, permitted capabilities = CAPS_ALL
TEST_EXECVE_NO_LOST_CAPS(rrr_no_lost_caps, root, root, root);
TEST_EXECVE_NO_LOST_CAPS(rrn_no_lost_caps, root, root, nobody);
TEST_EXECVE_NO_LOST_CAPS(nrr_no_lost_caps, nobody, root, root);
TEST_EXECVE_NO_LOST_CAPS(nrn_no_lost_caps, nobody, root, nobody);

// Effective UID != 0, Real UID = 0
//
// Final State:
// Effective capabilities = CAPS_NONE, permitted capabilities = CAPS_ALL
TEST_EXECVE_LOST_CAPS(rnr_lost_caps, root, nobody, root, CAPS_ALL);
TEST_EXECVE_LOST_CAPS(rnn_lost_caps, root, nobody, nobody, CAPS_ALL);

// Effective UID != 0, Real UID != 0
//
// Final State:
// Effective capabilities = CAPS_NONE, permitted capabilities = CAPS_NONE
TEST_EXECVE_LOST_CAPS(nnr_lost_caps, nobody, nobody, root, CAPS_NONE);
TEST_EXECVE_LOST_CAPS(nnn_lost_caps, nobody, nobody, nobody, CAPS_NONE);
