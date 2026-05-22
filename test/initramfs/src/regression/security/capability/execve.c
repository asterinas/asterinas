// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <stdint.h>
#include <sys/stat.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <sys/xattr.h>
#include <linux/capability.h>

#include "../../common/test.h"

static uid_t root = 0;
static uid_t nobody = 65534;

#define CAPS_ALL "000001ffffffffff"
#define CAPS_NET_BIND_SERVICE "0000000000000400"
#define CAPS_NONE "0000000000000000"

#define SECURITY_CAPABILITY_XATTR "security.capability"

#define AST_VFS_CAP_REVISION_2 0x02000000
#define AST_VFS_CAP_REVISION_3 0x03000000
#define AST_VFS_CAP_FLAGS_EFFECTIVE 0x00000001

struct ast_vfs_cap_data_v2 {
	uint32_t magic_etc;
	uint32_t permitted_low;
	uint32_t inheritable_low;
	uint32_t permitted_high;
	uint32_t inheritable_high;
};

struct ast_vfs_cap_data_v3 {
	uint32_t magic_etc;
	uint32_t permitted_low;
	uint32_t inheritable_low;
	uint32_t permitted_high;
	uint32_t inheritable_high;
	uint32_t rootid;
};

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

static char *copy_child_to_temp_exec(void)
{
	char *exec_path;
	char template[] = "/tmp/file_caps_execXXXXXX";
	char buffer[4096];
	int src_fd;
	int dst_fd;

	exec_path = CHECK_WITH(strdup(template), _ret != NULL);
	dst_fd = CHECK(mkstemp(exec_path));
	src_fd = CHECK(open(child_path, O_RDONLY));

	for (;;) {
		ssize_t read_len = CHECK(read(src_fd, buffer, sizeof(buffer)));
		ssize_t written = 0;

		if (read_len == 0) {
			break;
		}

		while (written < read_len) {
			written += CHECK(write(dst_fd, buffer + written,
					       read_len - written));
		}
	}

	CHECK(fchmod(dst_fd, 0755));
	CHECK(close(src_fd));
	CHECK(close(dst_fd));
	return exec_path;
}

static int run_file_cap_exec_test(const char *expected_effective,
				  const char *expected_permitted,
				  const char *expected_inheritable,
				  const void *xattr_value, size_t xattr_size)
{
	char *exec_path;
	pid_t pid;
	int ret;
	int saved_errno;
	int status;

	exec_path = copy_child_to_temp_exec();
	CHECK(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, xattr_value,
		       xattr_size, 0));

	pid = CHECK(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(exec_path, exec_path, expected_effective,
			    expected_permitted, expected_inheritable, NULL));
	}

	ret = waitpid(pid, &status, 0);
	saved_errno = errno;
	CHECK(unlink(exec_path));
	free(exec_path);

	if (ret != pid) {
		errno = saved_errno;
		return -1;
	}

	if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
		errno = ECHILD;
		return -1;
	}

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

FN_TEST(file_caps_v2_gain_effective_caps)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};

	TEST_SUCC(run_file_cap_exec_test(CAPS_NET_BIND_SERVICE,
					 CAPS_NET_BIND_SERVICE, CAPS_NONE,
					 &file_caps, sizeof(file_caps)));
}
END_TEST()

FN_TEST(file_caps_v2_gain_permitted_only_caps)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};

	TEST_SUCC(run_file_cap_exec_test(CAPS_NONE, CAPS_NET_BIND_SERVICE,
					 CAPS_NONE, &file_caps,
					 sizeof(file_caps)));
}
END_TEST()

FN_TEST(file_caps_v3_rootid_match)
{
	const struct ast_vfs_cap_data_v3 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_3 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
		.rootid = root,
	};

	TEST_SUCC(run_file_cap_exec_test(CAPS_NET_BIND_SERVICE,
					 CAPS_NET_BIND_SERVICE, CAPS_NONE,
					 &file_caps, sizeof(file_caps)));
}
END_TEST()

FN_TEST(file_caps_v3_rootid_mismatch)
{
	const struct ast_vfs_cap_data_v3 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_3 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
		.rootid = 1234,
	};

	TEST_SUCC(run_file_cap_exec_test(CAPS_NONE, CAPS_NONE, CAPS_NONE,
					 &file_caps, sizeof(file_caps)));
}
END_TEST()
