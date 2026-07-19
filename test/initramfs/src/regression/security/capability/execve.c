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
#define FILE_CAPS_EXEC_TEMPLATE "/tmp/file_caps_execXXXXXX"

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

static char *create_exec_with_file_caps(const void *xattr_value,
					size_t xattr_size)
{
	char *exec_path = copy_child_to_exec_template(FILE_CAPS_EXEC_TEMPLATE);

	CHECK(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, xattr_value,
		       xattr_size, 0));
	return exec_path;
}

enum file_mutation {
	FILE_MUTATION_FALLOCATE,
	FILE_MUTATION_WRITE,
	FILE_MUTATION_PWRITE,
	FILE_MUTATION_TRUNCATE,
	FILE_MUTATION_FTRUNCATE,
	FILE_MUTATION_OPEN_TRUNC,
};

static int mutate_file(const char *path, enum file_mutation mutation)
{
	int fd;
	int result;
	int mutation_errno;
	int close_result;

	if (mutation == FILE_MUTATION_TRUNCATE) {
		return truncate(path, 0);
	}

	if (mutation == FILE_MUTATION_OPEN_TRUNC) {
		fd = open(path, O_WRONLY | O_TRUNC);
		if (fd < 0) {
			return -1;
		}
		return close(fd);
	}

	fd = open(path,
		  mutation == FILE_MUTATION_FALLOCATE ? O_RDWR : O_WRONLY);
	if (fd < 0) {
		return -1;
	}

	switch (mutation) {
	case FILE_MUTATION_FALLOCATE:
		result = fallocate(
			fd, FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE, 0, 1);
		break;
	case FILE_MUTATION_WRITE:
		result = write(fd, "\x7f", 1) == 1 ? 0 : -1;
		break;
	case FILE_MUTATION_PWRITE:
		result = pwrite(fd, "\x7f", 1, 0) == 1 ? 0 : -1;
		break;
	case FILE_MUTATION_FTRUNCATE:
		result = ftruncate(fd, 0);
		break;
	default:
		errno = EINVAL;
		result = -1;
		break;
	}

	mutation_errno = errno;
	close_result = close(fd);
	if (result < 0) {
		errno = mutation_errno;
		return -1;
	}
	return close_result;
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
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_1 | VFS_CAP_FLAGS_EFFECTIVE,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
	};

	TEST_ERRNO(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			    XATTR_CAPS_SZ_1, 0),
		   EINVAL);
}
END_TEST()

FN_TEST(file_caps_v2_gain_effective_caps)
{
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2 | VFS_CAP_FLAGS_EFFECTIVE,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
	};
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(child_path, child_path, CAPS_NET_BIND_SERVICE,
			    CAPS_NET_BIND_SERVICE, CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(child_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

// File capabilities suppress the legacy setuid-root effective capability
// grant unless the xattr effective flag is set.
FN_TEST(file_caps_setuid_root_no_legacy_effective_caps)
{
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
	};
	pid_t pid;
	int status;

	TEST_SUCC(chmod(child_path, 04755));
	TEST_SUCC(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(child_path, child_path, CAPS_NONE,
			    CAPS_NET_BIND_SERVICE, CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(child_path, SECURITY_CAPABILITY_XATTR));
	TEST_SUCC(chmod(child_path, 0755));
}
END_TEST()

// A root process is not subject to the setuid-root + file-capability
// exception, so an effective UID of zero still notionally enables the file
// effective bit.
FN_TEST(file_caps_root_gets_legacy_effective_caps)
{
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
	};
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(clear_caps());
		CHECK(execl(child_path, child_path, CAPS_ALL, CAPS_ALL,
			    CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(child_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_v2_gain_permitted_only_caps)
{
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
	};
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(child_path, child_path, CAPS_NONE,
			    CAPS_NET_BIND_SERVICE, CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(child_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_v3_rootid_match)
{
	const struct vfs_ns_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_3 | VFS_CAP_FLAGS_EFFECTIVE,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
		.rootid = root,
	};
	struct vfs_cap_data read_caps;
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));
	TEST_RES(getxattr(child_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		 _ret == sizeof(read_caps));
	TEST_ERRNO(getxattr(child_path, SECURITY_CAPABILITY_XATTR, &read_caps,
			    sizeof(read_caps) - 1),
		   ERANGE);
	TEST_RES(getxattr(child_path, SECURITY_CAPABILITY_XATTR, &read_caps,
			  sizeof(read_caps)),
		 _ret == sizeof(read_caps) &&
			 read_caps.magic_etc == (VFS_CAP_REVISION_2 |
						 VFS_CAP_FLAGS_EFFECTIVE) &&
			 read_caps.data[0].permitted ==
				 (1U << CAP_NET_BIND_SERVICE));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(child_path, child_path, CAPS_NET_BIND_SERVICE,
			    CAPS_NET_BIND_SERVICE, CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(child_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_v3_rootid_mismatch)
{
	const struct vfs_ns_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_3 | VFS_CAP_FLAGS_EFFECTIVE,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
		.rootid = 1234,
	};
	struct vfs_ns_cap_data read_caps;
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));
	TEST_RES(getxattr(child_path, SECURITY_CAPABILITY_XATTR, NULL, 0),
		 _ret == sizeof(read_caps));
	TEST_RES(getxattr(child_path, SECURITY_CAPABILITY_XATTR, &read_caps,
			  sizeof(read_caps)),
		 _ret == sizeof(read_caps) &&
			 read_caps.magic_etc == (VFS_CAP_REVISION_3 |
						 VFS_CAP_FLAGS_EFFECTIVE) &&
			 read_caps.rootid == 1234);

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(child_path, child_path, CAPS_NONE, CAPS_NONE,
			    CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(child_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_execute_only)
{
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2 | VFS_CAP_FLAGS_EFFECTIVE,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
	};
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));
	TEST_SUCC(chmod(child_path, 0111));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(child_path, child_path, CAPS_NET_BIND_SERVICE,
			    CAPS_NET_BIND_SERVICE, CAPS_NONE, NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(child_path, SECURITY_CAPABILITY_XATTR));
	TEST_SUCC(chmod(child_path, 0755));
}
END_TEST()

FN_TEST(file_caps_inheritable_path)
{
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2,
		.data[0].inheritable = 1U << CAP_NET_BIND_SERVICE,
	};
	struct __user_cap_data_struct cap_data[2] = {};
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		read_cap_data(cap_data);
		cap_data[0].inheritable |= 1U << CAP_NET_BIND_SERVICE;
		write_cap_data(cap_data);
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK(execl(child_path, child_path, CAPS_NONE,
			    CAPS_NET_BIND_SERVICE, CAPS_NET_BIND_SERVICE,
			    NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(child_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_bounding_set_eperm)
{
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2 | VFS_CAP_FLAGS_EFFECTIVE,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
	};
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(prctl(PR_CAPBSET_DROP, CAP_NET_BIND_SERVICE, 0, 0, 0));
		CHECK(setresuid(nobody, nobody, nobody));
		CHECK_WITH(execl(child_path, child_path, CAPS_NONE, CAPS_NONE,
				 CAPS_NONE, NULL),
			   _ret == -1 && errno == EPERM);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(child_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_ignored_on_shebang_script)
{
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2 | VFS_CAP_FLAGS_EFFECTIVE,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
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
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2 | VFS_CAP_FLAGS_EFFECTIVE,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
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
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2 | VFS_CAP_FLAGS_EFFECTIVE,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
	};
	pid_t pid;
	int status;

	TEST_SUCC(setxattr(child_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			   sizeof(file_caps), 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		drop_capability(CAP_SETFCAP);
		CHECK_WITH(setxattr(child_path, SECURITY_CAPABILITY_XATTR,
				    &file_caps, sizeof(file_caps), 0),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(removexattr(child_path, SECURITY_CAPABILITY_XATTR),
			   _ret == -1 && errno == EPERM);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(removexattr(child_path, SECURITY_CAPABILITY_XATTR));
}
END_TEST()

FN_TEST(file_caps_modify_does_not_require_dac_write_permission)
{
	const struct vfs_cap_data file_caps = {
		.magic_etc = VFS_CAP_REVISION_2 | VFS_CAP_FLAGS_EFFECTIVE,
		.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,
	};
	char *exec_path = copy_child_to_exec_template(FILE_CAPS_EXEC_TEMPLATE);
	pid_t pid;
	int status;

	TEST_SUCC(chmod(exec_path, 0555));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		drop_capability(CAP_DAC_OVERRIDE);
		CHECK(setxattr(exec_path, SECURITY_CAPABILITY_XATTR, &file_caps,
			       sizeof(file_caps), 0));
		CHECK_WITH(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL,
				    0),
			   _ret == sizeof(file_caps));
		CHECK(removexattr(exec_path, SECURITY_CAPABILITY_XATTR));
		CHECK_WITH(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL,
				    0),
			   _ret == -1 && errno == ENODATA);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(unlink(exec_path));
	free(exec_path);
}
END_TEST()

FN_TEST(file_caps_reject_invalid_xattr_header)
{
	const uint32_t truncated_header = VFS_CAP_REVISION_2;
	const struct vfs_cap_data unsupported_revision = {
		.magic_etc = 0x04000000,
	};
	const struct vfs_cap_data unsupported_flags = {
		.magic_etc = VFS_CAP_REVISION_2 | 0x2,
	};
	const struct vfs_cap_data revision_length_mismatch = {
		.magic_etc = VFS_CAP_REVISION_3,
	};
	const struct vfs_ns_cap_data invalid_rootid = {
		.magic_etc = VFS_CAP_REVISION_3,
		.rootid = UINT32_MAX,
	};
	const struct {
		struct vfs_ns_cap_data caps;
		uint32_t trailing;
	} oversized_v3 = {
		.caps.magic_etc = VFS_CAP_REVISION_3,
	};

	TEST_ERRNO(setxattr(child_path, SECURITY_CAPABILITY_XATTR, NULL, 0, 0),
		   EINVAL);
	TEST_ERRNO(setxattr(child_path, SECURITY_CAPABILITY_XATTR,
			    &truncated_header, sizeof(truncated_header) - 1, 0),
		   EINVAL);
	TEST_ERRNO(setxattr(child_path, SECURITY_CAPABILITY_XATTR,
			    &unsupported_revision, sizeof(unsupported_revision),
			    0),
		   EINVAL);
	TEST_ERRNO(setxattr(child_path, SECURITY_CAPABILITY_XATTR,
			    &unsupported_flags, sizeof(unsupported_flags), 0),
		   EINVAL);
	TEST_ERRNO(setxattr(child_path, SECURITY_CAPABILITY_XATTR,
			    &revision_length_mismatch,
			    sizeof(revision_length_mismatch), 0),
		   EINVAL);
	TEST_ERRNO(setxattr(child_path, SECURITY_CAPABILITY_XATTR,
			    &invalid_rootid, sizeof(invalid_rootid), 0),
		   EINVAL);
	TEST_ERRNO(setxattr(child_path, SECURITY_CAPABILITY_XATTR,
			    &oversized_v3, sizeof(oversized_v3), 0),
		   EINVAL);
}
END_TEST()

#define TEST_FILE_CAPS_CLEARED_AFTER(name, mutation)                      \
	FN_TEST(name)                                                     \
	{                                                                 \
		const struct vfs_cap_data file_caps = {                   \
			.magic_etc = VFS_CAP_REVISION_2 |                 \
				     VFS_CAP_FLAGS_EFFECTIVE,             \
			.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,  \
		};                                                        \
		char *exec_path = create_exec_with_file_caps(             \
			&file_caps, sizeof(file_caps));                   \
                                                                          \
		TEST_SUCC(mutate_file(exec_path, mutation));              \
		TEST_ERRNO(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, \
				    NULL, 0),                             \
			   ENODATA);                                      \
                                                                          \
		TEST_SUCC(unlink(exec_path));                             \
		free(exec_path);                                          \
	}                                                                 \
	END_TEST()

#define TEST_FILE_CAPS_PRESERVED_AFTER_DENIED_MUTATION(name, mutation)        \
	FN_TEST(name)                                                         \
	{                                                                     \
		const struct vfs_cap_data file_caps = {                       \
			.magic_etc = VFS_CAP_REVISION_2 |                     \
				     VFS_CAP_FLAGS_EFFECTIVE,                 \
			.data[0].permitted = 1U << CAP_NET_BIND_SERVICE,      \
		};                                                            \
		char *exec_path =                                             \
			copy_child_to_exec_template(FILE_CAPS_EXEC_TEMPLATE); \
		pid_t pid;                                                    \
		int status;                                                   \
                                                                              \
		TEST_SUCC(chmod(exec_path, 0555));                            \
		TEST_SUCC(setxattr(exec_path, SECURITY_CAPABILITY_XATTR,      \
				   &file_caps, sizeof(file_caps), 0));        \
                                                                              \
		pid = TEST_SUCC(fork());                                      \
		if (pid == 0) {                                               \
			CHECK(setresuid(nobody, nobody, nobody));             \
			CHECK_WITH(mutate_file(exec_path, mutation),          \
				   _ret == -1 && errno == EACCES);            \
			_exit(EXIT_SUCCESS);                                  \
		}                                                             \
                                                                              \
		TEST_RES(waitpid(pid, &status, 0),                            \
			 _ret == pid && WIFEXITED(status) &&                  \
				 WEXITSTATUS(status) == 0);                   \
		TEST_RES(getxattr(exec_path, SECURITY_CAPABILITY_XATTR, NULL, \
				  0),                                         \
			 _ret > 0);                                           \
		TEST_SUCC(unlink(exec_path));                                 \
		free(exec_path);                                              \
	}                                                                     \
	END_TEST()

TEST_FILE_CAPS_CLEARED_AFTER(file_caps_cleared_after_fallocate,
			     FILE_MUTATION_FALLOCATE);
TEST_FILE_CAPS_CLEARED_AFTER(file_caps_cleared_after_write,
			     FILE_MUTATION_WRITE);
TEST_FILE_CAPS_CLEARED_AFTER(file_caps_cleared_after_pwrite,
			     FILE_MUTATION_PWRITE);
TEST_FILE_CAPS_CLEARED_AFTER(file_caps_cleared_after_truncate,
			     FILE_MUTATION_TRUNCATE);
TEST_FILE_CAPS_PRESERVED_AFTER_DENIED_MUTATION(
	file_caps_preserved_after_failed_truncate, FILE_MUTATION_TRUNCATE);
TEST_FILE_CAPS_CLEARED_AFTER(file_caps_cleared_after_ftruncate,
			     FILE_MUTATION_FTRUNCATE);
TEST_FILE_CAPS_CLEARED_AFTER(file_caps_cleared_after_open_trunc,
			     FILE_MUTATION_OPEN_TRUNC);
TEST_FILE_CAPS_PRESERVED_AFTER_DENIED_MUTATION(
	file_caps_preserved_after_failed_open_trunc, FILE_MUTATION_OPEN_TRUNC);

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
