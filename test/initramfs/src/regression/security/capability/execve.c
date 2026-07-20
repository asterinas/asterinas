// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/capability.h"
#include <fcntl.h>
#include <linux/falloc.h>
#include <stdbool.h>
#include <stdint.h>
#include <sys/mount.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <sys/xattr.h>

static uid_t root = 0;
static uid_t nobody = 65534;

#define CAPS_ALL "000001ffffffffff"
#define CAPS_NET_BIND_SERVICE "0000000000000400"
#define CAPS_NONE "0000000000000000"

#define SECURITY_CAPABILITY_XATTR "security.capability"

#define AST_VFS_CAP_REVISION_1 0x01000000
#define AST_VFS_CAP_REVISION_2 0x02000000
#define AST_VFS_CAP_REVISION_3 0x03000000
#define AST_VFS_CAP_FLAGS_EFFECTIVE 0x00000001

struct ast_vfs_cap_data_v1 {
	uint32_t magic_etc;
	uint32_t permitted;
	uint32_t inheritable;
};

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

static char *copy_child_to_exec_template(const char *template)
{
	char *exec_path;
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

static char *copy_child_to_temp_exec(void)
{
	return copy_child_to_exec_template("/tmp/file_caps_execXXXXXX");
}

static char *create_exec_with_file_caps(const void *xattr_value,
					size_t xattr_size)
{
	char *exec_path = copy_child_to_temp_exec();

	CHECK(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, xattr_value,
		       xattr_size, 0));
	return exec_path;
}

static mode_t setid_bits_after_child_pwrite(mode_t mode, gid_t file_gid,
					    bool drop_fsetid,
					    bool clear_supplementary_groups)
{
	char file_path[] = "/tmp/file_caps_setidXXXXXX";
	struct stat stat_buf;
	int fd = CHECK(mkstemp(file_path));
	pid_t pid;
	int status;

	CHECK(fchown(fd, -1, file_gid));
	CHECK(fchmod(fd, mode));

	pid = CHECK(fork());
	if (pid == 0) {
		if (clear_supplementary_groups) {
			CHECK(syscall(SYS_setgroups, 0, NULL));
		}
		if (drop_fsetid) {
			drop_capability(CAP_FSETID);
		}
		CHECK_WITH(pwrite(fd, "\x7f", 1, 0), _ret == 1);
		_exit(EXIT_SUCCESS);
	}

	CHECK_WITH(waitpid(pid, &status, 0), _ret == pid && WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	CHECK(fstat(fd, &stat_buf));
	CHECK(close(fd));
	CHECK(unlink(file_path));
	return stat_buf.st_mode & (S_ISUID | S_ISGID);
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

FN_TEST(file_caps_v1_write_rejected)
{
	const struct ast_vfs_cap_data_v1 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_1 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted = 1U << CAP_NET_BIND_SERVICE,
	};

	TEST_ERRNO(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			    sizeof(file_caps), 0),
		   EINVAL);
}
END_TEST()

FN_TEST(file_caps_v2_gain_effective_caps)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	const char *exec_path = child_path;
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(exec_path, exec_path, CAPS_NET_BIND_SERVICE,
			    CAPS_NET_BIND_SERVICE, CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(exec_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

// File capabilities suppress the legacy setuid-root effective capability
// grant unless the xattr effective flag is set.
FN_TEST(file_caps_setuid_root_no_legacy_effective_caps)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	const char *exec_path = child_path;
	pid_t pid;
	int status;

	TEST_SUCC(chmod(exec_path, 04755));
	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(exec_path, exec_path, CAPS_NONE,
			    CAPS_NET_BIND_SERVICE, CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(exec_path, SECURITY_CAPABILITY_XATTR));
	TEST_SUCC(chmod(exec_path, 0755));
}
END_TEST()

FN_TEST(file_caps_v2_gain_permitted_only_caps)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	const char *exec_path = child_path;
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(exec_path, exec_path, CAPS_NONE,
			    CAPS_NET_BIND_SERVICE, CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(exec_path, SECURITY_CAPABILITY_XATTR));
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
	struct ast_vfs_cap_data_v2 read_caps;
	const char *exec_path = child_path;
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));
	TEST_RES(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		 _ret == sizeof(read_caps));
	TEST_ERRNO(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, &read_caps,
			    sizeof(read_caps) - 1),
		   ERANGE);
	TEST_RES(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, &read_caps,
			  sizeof(read_caps)),
		 _ret == sizeof(read_caps) &&
			 read_caps.magic_etc == (AST_VFS_CAP_REVISION_2 |
						 AST_VFS_CAP_FLAGS_EFFECTIVE) &&
			 read_caps.permitted_low ==
				 (1U << CAP_NET_BIND_SERVICE));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(exec_path, exec_path, CAPS_NET_BIND_SERVICE,
			    CAPS_NET_BIND_SERVICE, CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(exec_path, SECURITY_CAPABILITY_XATTR));
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
	struct ast_vfs_cap_data_v3 read_caps;
	const char *exec_path = child_path;
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));
	TEST_RES(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		 _ret == sizeof(read_caps));
	TEST_RES(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, &read_caps,
			  sizeof(read_caps)),
		 _ret == sizeof(read_caps) &&
			 read_caps.magic_etc == (AST_VFS_CAP_REVISION_3 |
						 AST_VFS_CAP_FLAGS_EFFECTIVE) &&
			 read_caps.rootid == 1234);

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(exec_path, exec_path, CAPS_NONE, CAPS_NONE,
			    CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(exec_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_execute_only)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	const char *exec_path = child_path;
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));
	TEST_SUCC(chmod(exec_path, 0111));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(exec_path, exec_path, CAPS_NET_BIND_SERVICE,
			    CAPS_NET_BIND_SERVICE, CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(exec_path, SECURITY_CAPABILITY_XATTR));
	TEST_SUCC(chmod(exec_path, 0755));
}
END_TEST()

FN_TEST(file_caps_inheritable_path)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2,
		.inheritable_low = 1U << CAP_NET_BIND_SERVICE,
	};
	struct __user_cap_data_struct cap_data[2] = {};
	const char *exec_path = child_path;
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		read_cap_data(cap_data);
		cap_data[0].inheritable |= 1U << CAP_NET_BIND_SERVICE;
		write_cap_data(cap_data);
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(exec_path, exec_path, CAPS_NONE,
			    CAPS_NET_BIND_SERVICE, CAPS_NET_BIND_SERVICE,
			    NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(exec_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_bounding_set_eperm)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	const char *exec_path = child_path;
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(prctl(PR_CAPBSET_DROP, CAP_NET_BIND_SERVICE, 0, 0, 0));
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK_WITH(execl(exec_path, exec_path, CAPS_NONE, CAPS_NONE,
				 CAPS_NONE, NULL),
			   _ret == -1 && errno == EPERM);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(exec_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_ignored_on_shebang_script)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	char template[] = "/tmp/file_caps_scriptXXXXXX";
	char *script_path = TEST_RES(strdup(template), _ret != NULL);
	int script_fd = TEST_SUCC(mkstemp(script_path));
	pid_t pid;
	int status;

	TEST_RES(dprintf(script_fd, "#!%s %s %s %s\n", child_path, CAPS_NONE,
			 CAPS_NONE, CAPS_NONE),
		 _ret > 0);
	TEST_SUCC(fchmod(script_fd, 0755));
	TEST_SUCC(close(script_fd));
	TEST_SUCC(setxattr(script_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(script_path, script_path, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(unlink(script_path));
	free(script_path);
}
END_TEST()

FN_TEST(file_caps_ignored_on_nosuid_mount)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	char mount_template[] = "/tmp/file_caps_nosuidXXXXXX";
	char exec_template[4096];
	char *mount_path =
		TEST_RES(mkdtemp(mount_template), _ret == mount_template);
	char *exec_path;
	pid_t pid;
	int status;

	TEST_SUCC(mount("tmpfs", mount_path, "tmpfs", MS_NOSUID, NULL));
	TEST_RES(snprintf(exec_template, sizeof(exec_template), "%s/execXXXXXX",
			  mount_path),
		 _ret > 0 && (size_t)_ret < sizeof(exec_template));
	exec_path = copy_child_to_exec_template(exec_template);
	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(exec_path, exec_path, CAPS_NONE, CAPS_NONE,
			    CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(unlink(exec_path));
	free(exec_path);
	TEST_SUCC(umount(mount_path));
	TEST_SUCC(rmdir(mount_path));
}
END_TEST()

FN_TEST(file_caps_require_setfcap)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	const char *exec_path = child_path;
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		drop_capability(CAP_SETFCAP);
		CHECK_WITH(setxattr(exec_path, SECURITY_CAPABILITY_XATTR,
				    &file_caps, sizeof(file_caps), 0),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(removexattr(exec_path, SECURITY_CAPABILITY_XATTR),
			   _ret == -1 && errno == EPERM);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(exec_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_reject_invalid_xattr_header)
{
	const uint32_t truncated_header = AST_VFS_CAP_REVISION_2;
	const struct ast_vfs_cap_data_v2 unsupported_revision = {
		.magic_etc = 0x04000000,
	};
	const struct ast_vfs_cap_data_v2 unsupported_flags = {
		.magic_etc = AST_VFS_CAP_REVISION_2 | 0x2,
	};
	const struct ast_vfs_cap_data_v2 revision_length_mismatch = {
		.magic_etc = AST_VFS_CAP_REVISION_3,
	};
	const struct ast_vfs_cap_data_v3 invalid_rootid = {
		.magic_etc = AST_VFS_CAP_REVISION_3,
		.rootid = UINT32_MAX,
	};
	const char *exec_path = child_path;

	TEST_ERRNO(setxattr(exec_path, SECURITY_CAPABILITY_XATTR,
			    &truncated_header, sizeof(truncated_header) - 1, 0),
		   EINVAL);
	TEST_ERRNO(setxattr(exec_path, SECURITY_CAPABILITY_XATTR,
			    &unsupported_revision, sizeof(unsupported_revision),
			    0),
		   EINVAL);
	TEST_ERRNO(setxattr(exec_path, SECURITY_CAPABILITY_XATTR,
			    &unsupported_flags, sizeof(unsupported_flags), 0),
		   EINVAL);
	TEST_ERRNO(setxattr(exec_path, SECURITY_CAPABILITY_XATTR,
			    &revision_length_mismatch,
			    sizeof(revision_length_mismatch), 0),
		   EINVAL);
	TEST_ERRNO(setxattr(exec_path, SECURITY_CAPABILITY_XATTR,
			    &invalid_rootid, sizeof(invalid_rootid), 0),
		   EINVAL);
}
END_TEST()

FN_TEST(file_caps_cleared_after_fallocate)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	char *exec_path =
		create_exec_with_file_caps(&file_caps, sizeof(file_caps));
	int fd = TEST_SUCC(open(exec_path, O_RDWR));

	TEST_SUCC(fallocate(fd, FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE, 0,
			    1));
	TEST_ERRNO(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		   ENODATA);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(exec_path));
	free(exec_path);
}
END_TEST()

FN_TEST(file_caps_cleared_after_write)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	char *exec_path =
		create_exec_with_file_caps(&file_caps, sizeof(file_caps));
	int fd = TEST_SUCC(open(exec_path, O_WRONLY));

	TEST_RES(write(fd, "\x7f", 1), _ret == 1);
	TEST_ERRNO(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		   ENODATA);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(exec_path));
	free(exec_path);
}
END_TEST()

FN_TEST(file_caps_cleared_after_pwrite)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	char *exec_path =
		create_exec_with_file_caps(&file_caps, sizeof(file_caps));
	int fd = TEST_SUCC(open(exec_path, O_WRONLY));

	TEST_RES(pwrite(fd, "\x7f", 1, 0), _ret == 1);
	TEST_ERRNO(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		   ENODATA);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(exec_path));
	free(exec_path);
}
END_TEST()

FN_TEST(file_caps_cleared_after_truncate)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	char *exec_path =
		create_exec_with_file_caps(&file_caps, sizeof(file_caps));

	TEST_SUCC(truncate(exec_path, 0));
	TEST_ERRNO(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		   ENODATA);

	TEST_SUCC(unlink(exec_path));
	free(exec_path);
}
END_TEST()

FN_TEST(file_caps_preserved_after_failed_truncate)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	char *exec_path = copy_child_to_temp_exec();
	pid_t pid;
	int status;

	TEST_SUCC(chmod(exec_path, 0555));
	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK_WITH(truncate(exec_path, 0),
			   _ret == -1 && errno == EACCES);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_RES(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		 _ret > 0);
	TEST_SUCC(unlink(exec_path));
	free(exec_path);
}
END_TEST()

FN_TEST(file_caps_cleared_after_ftruncate)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	char *exec_path =
		create_exec_with_file_caps(&file_caps, sizeof(file_caps));
	int fd = TEST_SUCC(open(exec_path, O_WRONLY));

	TEST_SUCC(ftruncate(fd, 0));
	TEST_ERRNO(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		   ENODATA);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(exec_path));
	free(exec_path);
}
END_TEST()

FN_TEST(file_caps_cleared_after_open_trunc)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	char *exec_path =
		create_exec_with_file_caps(&file_caps, sizeof(file_caps));
	int fd = TEST_SUCC(open(exec_path, O_WRONLY | O_TRUNC));

	TEST_ERRNO(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		   ENODATA);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(exec_path));
	free(exec_path);
}
END_TEST()

FN_TEST(file_caps_preserved_after_failed_open_trunc)
{
	const struct ast_vfs_cap_data_v2 file_caps = {
		.magic_etc = AST_VFS_CAP_REVISION_2 |
			     AST_VFS_CAP_FLAGS_EFFECTIVE,
		.permitted_low = 1U << CAP_NET_BIND_SERVICE,
	};
	char *exec_path = copy_child_to_temp_exec();
	pid_t pid;
	int status;

	TEST_SUCC(chmod(exec_path, 0555));
	TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK_WITH(open(exec_path, O_WRONLY | O_TRUNC),
			   _ret == -1 && errno == EACCES);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_RES(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		 _ret > 0);
	TEST_SUCC(unlink(exec_path));
	free(exec_path);
}
END_TEST()

FN_TEST(file_caps_setid_clearing_honors_fsetid_and_file_group)
{
	TEST_RES(setid_bits_after_child_pwrite(06750, getegid(), false, false),
		 _ret == (S_ISUID | S_ISGID));
	TEST_RES(setid_bits_after_child_pwrite(06750, getegid(), true, false),
		 _ret == 0);
	TEST_RES(setid_bits_after_child_pwrite(02640, getegid(), true, false),
		 _ret == S_ISGID);
	TEST_RES(setid_bits_after_child_pwrite(02640, nobody, true, true),
		 _ret == 0);
}
END_TEST()
