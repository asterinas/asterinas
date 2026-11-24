// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <grp.h>
#include <linux/securebits.h>
#include <sys/fsuid.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/capability.h"
#include "../../common/test.h"

#define TEST_DIR "/tmp/access_credentials"
#define OWNER_FILE TEST_DIR "/owner"
#define GROUP_FILE TEST_DIR "/group"
#define PRIVATE_DIR TEST_DIR "/private"
#define PRIVATE_FILE PRIVATE_DIR "/file"
#define TEST_UID 5000
#define TEST_GID 5000

static void create_file(const char *path, mode_t mode)
{
	int fd = CHECK(open(path, O_CREAT | O_EXCL | O_WRONLY, 0600));
	CHECK(fchmod(fd, mode));
	CHECK(close(fd));
}

static void check_access(const char *path, int mode, int flags,
			 int expected_errno)
{
	long result = syscall(SYS_faccessat2, AT_FDCWD, path, mode, flags);
	int error = errno;
	if (expected_errno ? result != -1 || error != expected_errno :
			     result != 0) {
		fprintf(stderr,
			"faccessat2: path=%s mode=%#x flags=%#x result=%ld "
			"errno=%d expected_errno=%d\n",
			path, mode, flags, result, error, expected_errno);
	}
	errno = error;
	CHECK_WITH(result, expected_errno ?
				   _ret == -1 && error == expected_errno :
				   _ret == 0);
}

static void wait_child(pid_t pid)
{
	int status;
	CHECK_WITH(waitpid(pid, &status, 0),
		   _ret == pid && WIFEXITED(status) &&
			   WEXITSTATUS(status) == EXIT_SUCCESS);
}

FN_SETUP(fixtures)
{
	CHECK(mkdir(TEST_DIR, 0755));
	CHECK(chmod(TEST_DIR, 0755));
	create_file(OWNER_FILE, 0400);
	create_file(GROUP_FILE, 0040);
	CHECK(chown(GROUP_FILE, 0, TEST_GID));
	CHECK(mkdir(PRIVATE_DIR, 0700));
	create_file(PRIVATE_FILE, 0444);
}
END_SETUP()

FN_TEST(real_and_effective_user_ids)
{
	const uid_t real_ids[] = { TEST_UID, 0 };
	const uid_t effective_ids[] = { 0, TEST_UID };

	for (size_t i = 0; i < 2; i++) {
		pid_t pid = TEST_SUCC(fork());
		if (pid == 0) {
			CHECK(setgroups(0, NULL));
			CHECK(setresgid(TEST_GID, TEST_GID, 0));
			CHECK(setresuid(real_ids[i], effective_ids[i], 0));

			int real_error = i == 0 ? EACCES : 0;
			int effective_error = i == 0 ? 0 : EACCES;
			CHECK_WITH(access(OWNER_FILE, R_OK),
				   real_error ?
					   _ret == -1 && errno == real_error :
					   _ret == 0);
			CHECK_WITH(syscall(SYS_faccessat, AT_FDCWD, OWNER_FILE,
					   R_OK),
				   real_error ?
					   _ret == -1 && errno == real_error :
					   _ret == 0);
			check_access(OWNER_FILE, R_OK, 0, real_error);
			check_access(OWNER_FILE, R_OK, AT_EACCESS,
				     effective_error);

			// F_OK must still check search permission on parent directories.
			check_access(PRIVATE_FILE, F_OK, 0, real_error);
			check_access(PRIVATE_FILE, F_OK, AT_EACCESS,
				     effective_error);
			check_access(OWNER_FILE, F_OK, 0, 0);
			_exit(EXIT_SUCCESS);
		}
		wait_child(pid);
	}
}
END_TEST()

FN_TEST(real_and_effective_group_ids)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setgroups(0, NULL));
		CHECK(setresgid(TEST_GID, TEST_GID + 1, 0));
		CHECK(setresuid(TEST_UID, TEST_UID, 0));
		check_access(GROUP_FILE, R_OK, 0, 0);
		check_access(GROUP_FILE, R_OK, AT_EACCESS, EACCES);
		_exit(EXIT_SUCCESS);
	}
	wait_child(pid);
}
END_TEST()

FN_TEST(supplementary_groups_apply_to_both_modes)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		gid_t group = TEST_GID;
		CHECK(setgroups(1, &group));
		CHECK(setresgid(TEST_GID + 1, TEST_GID + 1, 0));
		CHECK(setresuid(TEST_UID, TEST_UID, 0));
		check_access(GROUP_FILE, R_OK, 0, 0);
		check_access(GROUP_FILE, R_OK, AT_EACCESS, 0);
		_exit(EXIT_SUCCESS);
	}
	wait_child(pid);
}
END_TEST()

FN_TEST(effective_access_uses_filesystem_ids)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setgroups(0, NULL));
		CHECK(setresgid(TEST_GID, TEST_GID, 0));
		CHECK(setresuid(TEST_UID, TEST_UID, 0));
		setfsuid(0);
		CHECK_WITH(setfsuid(-1), _ret == 0);
		check_access(OWNER_FILE, R_OK, 0, EACCES);
		check_access(OWNER_FILE, R_OK, AT_EACCESS, 0);
		_exit(EXIT_SUCCESS);
	}
	wait_child(pid);
}
END_TEST()

FN_TEST(real_root_uses_permitted_capabilities)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		struct __user_cap_data_struct caps[2];
		read_cap_data(caps);
		caps[0].effective = 0;
		caps[1].effective = 0;
		write_cap_data(caps);
		check_access(PRIVATE_FILE, R_OK, 0, 0);
		// OWNER_FILE is owned by root, so use a group-only file instead.
		check_access(GROUP_FILE, R_OK, AT_EACCESS, EACCES);
		check_access(GROUP_FILE, R_OK, 0, 0);
		_exit(EXIT_SUCCESS);
	}
	wait_child(pid);
}
END_TEST()

FN_TEST(no_setuid_fixup_uses_effective_capabilities)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(prctl(PR_SET_SECUREBITS, SECBIT_NO_SETUID_FIXUP));
		CHECK(setresgid(TEST_GID, TEST_GID, 0));
		CHECK(setresuid(TEST_UID, TEST_UID, 0));
		// Set the capability state explicitly to isolate the access check
		// from capability fixups performed by setresuid.
		struct __user_cap_data_struct caps[2];
		read_cap_data(caps);
		caps[0].effective = caps[0].permitted;
		caps[1].effective = caps[1].permitted;
		write_cap_data(caps);
		check_access(OWNER_FILE, R_OK, 0, 0);
		check_access(OWNER_FILE, R_OK, AT_EACCESS, 0);
		_exit(EXIT_SUCCESS);
	}
	wait_child(pid);
}
END_TEST()

FN_TEST(read_search_capability_does_not_allow_regular_file_execution)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(prctl(PR_SET_SECUREBITS, SECBIT_NO_SETUID_FIXUP));
		CHECK(setresuid(TEST_UID, TEST_UID, 0));
		struct __user_cap_data_struct caps[2] = {};
		caps[0].effective = 1U << CAP_DAC_READ_SEARCH;
		caps[0].permitted = 1U << CAP_DAC_READ_SEARCH;
		write_cap_data(caps);
		check_access(PRIVATE_FILE, R_OK, 0, 0);
		check_access(PRIVATE_FILE, R_OK, AT_EACCESS, 0);
		check_access(PRIVATE_FILE, X_OK, 0, EACCES);
		check_access(PRIVATE_FILE, W_OK, AT_EACCESS, EACCES);
		_exit(EXIT_SUCCESS);
	}
	wait_child(pid);
}
END_TEST()

FN_TEST(empty_and_relative_paths_use_selected_credentials)
{
	int fd = TEST_SUCC(open(OWNER_FILE, O_PATH));
	int dirfd = TEST_SUCC(open(TEST_DIR, O_PATH | O_DIRECTORY));
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setgroups(0, NULL));
		CHECK(setresuid(TEST_UID, 0, 0));
		CHECK_WITH(syscall(SYS_faccessat2, fd, "", R_OK, AT_EMPTY_PATH),
			   _ret == -1 && errno == EACCES);
		CHECK(syscall(SYS_faccessat2, fd, "", R_OK,
			      AT_EMPTY_PATH | AT_EACCESS));
		CHECK_WITH(syscall(SYS_faccessat2, dirfd, "owner", R_OK, 0),
			   _ret == -1 && errno == EACCES);
		CHECK(syscall(SYS_faccessat2, dirfd, "owner", R_OK,
			      AT_EACCESS));
		_exit(EXIT_SUCCESS);
	}
	wait_child(pid);
	TEST_SUCC(close(dirfd));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(symlink_resolution_uses_selected_credentials)
{
	TEST_SUCC(symlink("private/file", TEST_DIR "/link"));
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresuid(TEST_UID, 0, 0));
		check_access(TEST_DIR "/link", R_OK, 0, EACCES);
		check_access(TEST_DIR "/link", R_OK, AT_EACCESS, 0);
		check_access(TEST_DIR "/link", R_OK, AT_SYMLINK_NOFOLLOW, 0);
		_exit(EXIT_SUCCESS);
	}
	wait_child(pid);
	TEST_SUCC(unlink(TEST_DIR "/link"));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(unlink(PRIVATE_FILE));
	CHECK(rmdir(PRIVATE_DIR));
	CHECK(unlink(GROUP_FILE));
	CHECK(unlink(OWNER_FILE));
	CHECK(rmdir(TEST_DIR));
}
END_SETUP()
