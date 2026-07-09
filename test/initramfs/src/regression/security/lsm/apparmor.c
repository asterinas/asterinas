// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <ctype.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#ifndef RENAME_EXCHANGE
#define RENAME_EXCHANGE (1 << 1)
#endif

#define RAW_O_TMPFILE 020000000

#ifndef __O_TMPFILE
#define __O_TMPFILE RAW_O_TMPFILE
#endif

#ifndef O_TMPFILE
#define O_TMPFILE (__O_TMPFILE | O_DIRECTORY)
#endif

#define CMDLINE_BUFFER_SIZE 4096
#define FILE_BUFFER_SIZE 4096

static const char *CMDLINE_PATH = "/proc/cmdline";
static const char *APPARMOR_PROC_DIR_PATH = "/proc/sys/kernel/apparmor";
static const char *APPARMOR_PROC_PROFILES_PATH =
	"/proc/sys/kernel/apparmor/profiles";
static const char *APPARMOR_PROC_CURRENT_PATH =
	"/proc/sys/kernel/apparmor/current";
static const char *APPARMOR_PROC_LOAD_PATH = "/proc/sys/kernel/apparmor/load";
static const char *APPARMOR_ATTR_CURRENT_PATH = "/proc/self/attr/current";
static const char *APPARMOR_ATTR_EXEC_PATH = "/proc/self/attr/exec";
static const char *APPARMOR_ATTR_PREV_PATH = "/proc/self/attr/prev";
static const char *SECURITYFS_MOUNT_DIR = "/tmp/apparmor-securityfs";
static const char *SECURITYFS_APPARMOR_DIR =
	"/tmp/apparmor-securityfs/apparmor";
static const char *SECURITYFS_APPARMOR_PROFILES =
	"/tmp/apparmor-securityfs/apparmor/profiles";
static const char *SECURITYFS_APPARMOR_ABI =
	"/tmp/apparmor-securityfs/apparmor/features/abi";
static const char *SECURITYFS_APPARMOR_PERMSTABLE =
	"/tmp/apparmor-securityfs/apparmor/features/policy/permstable32";
static const char *SECURITYFS_APPARMOR_FILE_MASK =
	"/tmp/apparmor-securityfs/apparmor/features/file/mask";
static const char *SECURITYFS_APPARMOR_DOMAIN_STACK =
	"/tmp/apparmor-securityfs/apparmor/features/domain/stack";

static bool securityfs_mounted;

static const char *POLICY_MODE_PROFILE_ENFORCE = "asterinas-aa-mode-enforce";
static const char *POLICY_MODE_PROFILE_COMPLAIN = "asterinas-aa-mode-complain";
static const char *POLICY_MODE_PROFILE_COMPLAIN_IMPLICIT =
	"asterinas-aa-mode-complain-implicit";
static const char *POLICY_REPLACE_PROFILE_NAME = "asterinas-aa-replace-remove";
static const char *POLICY_MODE_PATH = "/tmp/aa-policy-mode";
static const char *FILE_HOOK_PROFILE_NAME = "asterinas-aa-file-hooks";
static const char *FILE_HOOK_PREOPENED_PATH = "/tmp/aa-file-preopened";
static const char *FILE_HOOK_ACCESS_PATH = "/tmp/aa-file-access";
static const char *FILE_HOOK_MMAP_PATH = "/tmp/aa-file-mmap";
static const char *FILE_HOOK_GETATTR_PATH = "/tmp/aa-file-getattr";
static const char *FILE_HOOK_READLINK_PATH = "/tmp/aa-file-readlink";
static const char *FILE_HOOK_RECEIVE_PATH = "/tmp/aa-file-receive";
static const char *FILE_HOOK_SYMLINK_TARGET = "/tmp/aa-file-link-target";
static const char *FILE_HOOK_RENAME_SOURCE_PATH = "/tmp/aa-file-rename-source";
static const char *FILE_HOOK_RENAME_TARGET_PATH = "/tmp/aa-file-rename-target";
static const char *FILE_HOOK_EXCHANGE_SOURCE_PATH =
	"/tmp/aa-file-exchange-source";
static const char *FILE_HOOK_EXCHANGE_TARGET_PATH =
	"/tmp/aa-file-exchange-target";
static const char *EXEC_HELPER_PATH = "/test/security/lsm/apparmor_exec_helper";
static const char *CHANGE_SOURCE_PROFILE_NAME = "asterinas-aa-change-source";
static const char *CHANGE_TARGET_PROFILE_NAME = "asterinas-aa-change-target";
static const char *ONEXEC_TARGET_PROFILE_NAME = "asterinas-aa-onexec-target";
static const char *EXEC_UNSAFE_SOURCE_PROFILE_NAME =
	"asterinas-aa-unsafe-source";
static const char *EXEC_SOURCE_PROFILE_NAME = "asterinas-aa-exec-source";
static const char *EXEC_IX_SOURCE_PROFILE_NAME = "asterinas-aa-ix-source";
static const char *EXEC_UX_UNSAFE_SOURCE_PROFILE_NAME =
	"asterinas-aa-ux-unsafe-source";
static const char *EXEC_UX_SOURCE_PROFILE_NAME = "asterinas-aa-ux-source";
static const char *EXEC_CHILD_SOURCE_PROFILE_NAME = "asterinas-aa-child-source";
static const char *EXEC_CX_SOURCE_PROFILE_NAME = "asterinas-aa-cx-source";
static const char *EXEC_BAD_CHILD_SOURCE_PROFILE_NAME =
	"asterinas-aa-bad-child-source";
#define EXEC_CHILD_TARGET_PROFILE_NAME "asterinas-aa-child-source//target"
#define EXEC_CX_TARGET_PROFILE_NAME "asterinas-aa-cx-source//target"
static const char *ONEXEC_OUTPUT_PATH = "/tmp/aa-onexec-current";
static const char *EXEC_UNSAFE_OUTPUT_PATH = "/tmp/aa-unsafe-current";
static const char *EXEC_UNSAFE_SECURE_PATH = "/tmp/aa-unsafe-secure";
static const char *EXEC_TRANSITION_OUTPUT_PATH = "/tmp/aa-exec-current";
static const char *EXEC_TRANSITION_SECURE_PATH = "/tmp/aa-exec-secure";
static const char *EXEC_IX_OUTPUT_PATH = "/tmp/aa-ix-current";
static const char *EXEC_IX_SECURE_PATH = "/tmp/aa-ix-secure";
static const char *EXEC_UX_UNSAFE_OUTPUT_PATH = "/tmp/aa-ux-unsafe-current";
static const char *EXEC_UX_UNSAFE_SECURE_PATH = "/tmp/aa-ux-unsafe-secure";
static const char *EXEC_UX_OUTPUT_PATH = "/tmp/aa-ux-current";
static const char *EXEC_UX_SECURE_PATH = "/tmp/aa-ux-secure";
static const char *EXEC_CHILD_OUTPUT_PATH = "/tmp/aa-child-current";
static const char *EXEC_CHILD_SECURE_PATH = "/tmp/aa-child-secure";
static const char *EXEC_CX_OUTPUT_PATH = "/tmp/aa-cx-current";
static const char *EXEC_CX_SECURE_PATH = "/tmp/aa-cx-secure";
static const char *EXEC_DENIED_OUTPUT_PATH = "/tmp/aa-denied-current";

static void read_cmdline(char cmdline[CMDLINE_BUFFER_SIZE])
{
	int fd = CHECK(open(CMDLINE_PATH, O_RDONLY));
	ssize_t len = CHECK(read(fd, cmdline, CMDLINE_BUFFER_SIZE - 1));

	cmdline[len] = '\0';
	CHECK(close(fd));
}

static char *trim(char *string)
{
	char *end;

	while (isspace((unsigned char)*string)) {
		string++;
	}

	end = string + strlen(string);
	while (end > string && isspace((unsigned char)*(end - 1))) {
		end--;
	}
	*end = '\0';

	return string;
}

static bool find_effective_param(const char *prefix,
				 char value[CMDLINE_BUFFER_SIZE])
{
	char cmdline[CMDLINE_BUFFER_SIZE];
	char *saveptr = NULL;
	size_t prefix_len = strlen(prefix);
	bool found = false;

	read_cmdline(cmdline);
	for (char *token = strtok_r(cmdline, " \n", &saveptr); token;
	     token = strtok_r(NULL, " \n", &saveptr)) {
		if (strncmp(token, prefix, prefix_len) != 0) {
			continue;
		}

		CHECK_WITH(snprintf(value, CMDLINE_BUFFER_SIZE, "%s",
				    token + prefix_len),
			   _ret >= 0 && _ret < CMDLINE_BUFFER_SIZE);
		found = true;
	}

	return found;
}

static bool module_list_contains(const char *list, const char *module_name)
{
	char list_copy[CMDLINE_BUFFER_SIZE];
	char *saveptr = NULL;

	CHECK_WITH(snprintf(list_copy, sizeof(list_copy), "%s", list),
		   _ret >= 0 && _ret < (int)sizeof(list_copy));
	for (char *module = strtok_r(list_copy, ",", &saveptr); module;
	     module = strtok_r(NULL, ",", &saveptr)) {
		if (strcmp(trim(module), module_name) == 0) {
			return true;
		}
	}

	return false;
}

static bool expect_apparmor_enabled(void)
{
	char lsm_param[CMDLINE_BUFFER_SIZE] = "";
	char security_param[CMDLINE_BUFFER_SIZE] = "";

	if (find_effective_param("lsm=", lsm_param)) {
		return module_list_contains(lsm_param, "apparmor");
	}

	if (find_effective_param("security=", security_param)) {
		return strcmp(trim(security_param), "apparmor") == 0;
	}

	return false;
}

static int stat_file_type(const char *path, mode_t file_type)
{
	struct stat statbuf;

	if (stat(path, &statbuf) < 0) {
		return -1;
	}

	if ((statbuf.st_mode & S_IFMT) != file_type) {
		errno = EINVAL;
		return -1;
	}

	return 0;
}

static int read_text_file(const char *path, char *buffer, size_t buffer_size)
{
	int fd;
	ssize_t len;

	if (buffer_size == 0) {
		errno = EINVAL;
		return -1;
	}

	fd = open(path, O_RDONLY);
	if (fd < 0) {
		return -1;
	}

	len = read(fd, buffer, buffer_size - 1);
	if (len < 0) {
		int saved_errno = errno;

		close(fd);
		errno = saved_errno;
		return -1;
	}

	buffer[len] = '\0';
	if (close(fd) < 0) {
		return -1;
	}

	return (int)len;
}

static int write_text_file(const char *path, const char *text)
{
	int fd = open(path, O_WRONLY);
	size_t len = strlen(text);
	size_t written = 0;

	if (fd < 0) {
		return -1;
	}

	while (written < len) {
		ssize_t count = write(fd, text + written, len - written);

		if (count < 0) {
			int saved_errno = errno;

			close(fd);
			errno = saved_errno;
			return -1;
		}
		written += (size_t)count;
	}

	if (close(fd) < 0) {
		return -1;
	}

	return 0;
}

static int create_text_file(const char *path, const char *text)
{
	int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
	size_t len = strlen(text);
	size_t written = 0;

	if (fd < 0) {
		return -1;
	}

	while (written < len) {
		ssize_t count = write(fd, text + written, len - written);

		if (count < 0) {
			int saved_errno = errno;

			close(fd);
			errno = saved_errno;
			return -1;
		}
		written += (size_t)count;
	}

	if (close(fd) < 0) {
		return -1;
	}

	return 0;
}

static int read_file_errno(const char *path)
{
	char buffer[FILE_BUFFER_SIZE];
	int fd = open(path, O_RDONLY);
	int saved_errno = 0;

	if (fd < 0) {
		saved_errno = errno;
		errno = 0;
		return saved_errno;
	}

	if (read(fd, buffer, sizeof(buffer)) < 0) {
		saved_errno = errno;
	}

	if (close(fd) < 0 && saved_errno == 0) {
		saved_errno = errno;
	}

	errno = 0;
	return saved_errno;
}

static int read_file_contains(const char *path, const char *expected)
{
	char buffer[FILE_BUFFER_SIZE];

	if (read_text_file(path, buffer, sizeof(buffer)) < 0) {
		return -1;
	}

	if (strstr(buffer, expected) == NULL) {
		errno = EINVAL;
		return -1;
	}

	return 0;
}

static int read_file_equals(const char *path, const char *expected)
{
	char buffer[FILE_BUFFER_SIZE];

	if (read_text_file(path, buffer, sizeof(buffer)) < 0) {
		return -1;
	}

	if (strcmp(buffer, expected) != 0) {
		errno = EINVAL;
		return -1;
	}

	return 0;
}

static int read_file_not_contains(const char *path, const char *unexpected)
{
	char buffer[FILE_BUFFER_SIZE];

	if (read_text_file(path, buffer, sizeof(buffer)) < 0) {
		return -1;
	}

	if (strstr(buffer, unexpected) != NULL) {
		errno = EINVAL;
		return -1;
	}

	return 0;
}

static int mount_securityfs(void)
{
	if (mkdir(SECURITYFS_MOUNT_DIR, 0755) < 0 && errno != EEXIST) {
		return -1;
	}

	if (mount("none", SECURITYFS_MOUNT_DIR, "securityfs", 0, "") < 0) {
		return -1;
	}

	securityfs_mounted = true;
	return 0;
}

static void cleanup_securityfs_mount(void)
{
	if (securityfs_mounted) {
		umount2(SECURITYFS_MOUNT_DIR, MNT_DETACH);
		securityfs_mounted = false;
	}
	rmdir(SECURITYFS_MOUNT_DIR);
}

static int send_fd(int socket_fd, int fd)
{
	char byte = 'x';
	struct iovec iov = {
		.iov_base = &byte,
		.iov_len = sizeof(byte),
	};
	char control[CMSG_SPACE(sizeof(int))];
	struct msghdr message = {
		.msg_iov = &iov,
		.msg_iovlen = 1,
		.msg_control = control,
		.msg_controllen = sizeof(control),
	};
	struct cmsghdr *cmsg;

	memset(control, 0, sizeof(control));
	cmsg = CMSG_FIRSTHDR(&message);
	cmsg->cmsg_level = SOL_SOCKET;
	cmsg->cmsg_type = SCM_RIGHTS;
	cmsg->cmsg_len = CMSG_LEN(sizeof(int));
	memcpy(CMSG_DATA(cmsg), &fd, sizeof(fd));
	message.msg_controllen = cmsg->cmsg_len;

	return sendmsg(socket_fd, &message, 0);
}

static int recv_fd(void)
{
	char byte;
	struct iovec iov = {
		.iov_base = &byte,
		.iov_len = sizeof(byte),
	};
	char control[CMSG_SPACE(sizeof(int))];
	struct msghdr message = {
		.msg_iov = &iov,
		.msg_iovlen = 1,
		.msg_control = control,
		.msg_controllen = sizeof(control),
	};

	if (recvmsg(0, &message, 0) < 0) {
		return -1;
	}

	return 0;
}

static int expect_eacces(int result)
{
	if (result >= 0) {
		errno = 0;
		return -1;
	}
	if (errno != EACCES) {
		return -1;
	}
	errno = 0;
	return 0;
}

static int run_read_policy_mode_child(const char *profile_name,
				      bool expect_denied)
{
	int fd;

	if (write_text_file(APPARMOR_PROC_CURRENT_PATH, profile_name) < 0) {
		return 1;
	}

	fd = open(POLICY_MODE_PATH, O_RDONLY);
	if (expect_denied) {
		if (fd >= 0) {
			close(fd);
			return 2;
		}
		if (errno != EACCES) {
			return 3;
		}
		errno = 0;
		return 0;
	}

	if (fd < 0) {
		return 4;
	}
	if (close(fd) < 0) {
		return 5;
	}

	return 0;
}

static int run_file_hook_child(void)
{
	char buffer[16];
	struct stat statbuf;
	int preopened_fd = open(FILE_HOOK_PREOPENED_PATH, O_RDONLY);
	int mmap_fd = open(FILE_HOOK_MMAP_PATH, O_RDONLY);
	int receive_fd = open(FILE_HOOK_RECEIVE_PATH, O_RDONLY);
	int sockets[2];
	void *mapping;

	if (preopened_fd < 0 || mmap_fd < 0 || receive_fd < 0) {
		return 1;
	}
	if (socketpair(AF_UNIX, SOCK_STREAM, 0, sockets) < 0) {
		return 2;
	}
	if (send_fd(sockets[0], receive_fd) < 0) {
		return 3;
	}
	if (dup2(sockets[1], 0) < 0) {
		return 4;
	}
	if (write_text_file(APPARMOR_PROC_CURRENT_PATH,
			    FILE_HOOK_PROFILE_NAME) < 0) {
		return 5;
	}

	if (expect_eacces(read(preopened_fd, buffer, sizeof(buffer))) < 0) {
		return 10;
	}
	if (expect_eacces(access(FILE_HOOK_ACCESS_PATH, R_OK)) < 0) {
		return 11;
	}
	if (expect_eacces(open("/tmp", O_TMPFILE | O_RDWR, 0600)) < 0) {
		return 12;
	}
	if (expect_eacces(stat(FILE_HOOK_GETATTR_PATH, &statbuf)) < 0) {
		return 13;
	}
	if (expect_eacces(readlink(FILE_HOOK_READLINK_PATH, buffer,
				   sizeof(buffer))) < 0) {
		return 14;
	}

	mapping = mmap(NULL, 4096, PROT_READ | PROT_EXEC, MAP_PRIVATE, mmap_fd,
		       0);
	if (mapping != MAP_FAILED) {
		munmap(mapping, 4096);
		return 15;
	}
	if (errno != EACCES) {
		return 16;
	}
	errno = 0;

	mapping = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, mmap_fd, 0);
	if (mapping == MAP_FAILED) {
		return 17;
	}
	if (expect_eacces(mprotect(mapping, 4096, PROT_READ | PROT_EXEC)) < 0) {
		munmap(mapping, 4096);
		return 18;
	}
	munmap(mapping, 4096);

	if (expect_eacces(recv_fd()) < 0) {
		return 19;
	}
	if (expect_eacces(rename(FILE_HOOK_RENAME_SOURCE_PATH,
				 FILE_HOOK_RENAME_TARGET_PATH)) < 0) {
		return 20;
	}
#ifdef SYS_renameat2
	if (expect_eacces(syscall(SYS_renameat2, AT_FDCWD,
				  FILE_HOOK_EXCHANGE_SOURCE_PATH, AT_FDCWD,
				  FILE_HOOK_EXCHANGE_TARGET_PATH,
				  RENAME_EXCHANGE)) < 0) {
		return 21;
	}
#endif

	return 0;
}

static int wait_for_child_success(pid_t pid)
{
	int status;

	if (waitpid(pid, &status, 0) < 0) {
		return -1;
	}
	if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
		errno = ECHILD;
		return -1;
	}

	return 0;
}

static int run_change_profile_child(void)
{
	if (write_text_file(APPARMOR_ATTR_CURRENT_PATH,
			    CHANGE_SOURCE_PROFILE_NAME) < 0) {
		return 1;
	}
	if (read_file_equals(APPARMOR_ATTR_CURRENT_PATH,
			     "asterinas-aa-change-source\n") < 0) {
		return 2;
	}
	if (write_text_file(APPARMOR_ATTR_CURRENT_PATH,
			    CHANGE_TARGET_PROFILE_NAME) < 0) {
		return 3;
	}
	if (read_file_equals(APPARMOR_ATTR_CURRENT_PATH,
			     "asterinas-aa-change-target\n") < 0) {
		return 4;
	}
	if (read_file_equals(APPARMOR_ATTR_PREV_PATH,
			     "asterinas-aa-change-source\n") < 0) {
		return 5;
	}
	if (expect_eacces(write_text_file(APPARMOR_ATTR_EXEC_PATH,
					  ONEXEC_TARGET_PROFILE_NAME)) < 0) {
		return 6;
	}
	if (expect_eacces(write_text_file(APPARMOR_ATTR_CURRENT_PATH,
					  CHANGE_SOURCE_PROFILE_NAME)) < 0) {
		return 7;
	}

	return 0;
}

static int run_onexec_child(void)
{
	if (write_text_file(APPARMOR_ATTR_CURRENT_PATH,
			    CHANGE_SOURCE_PROFILE_NAME) < 0) {
		return 1;
	}
	if (write_text_file(APPARMOR_ATTR_EXEC_PATH,
			    ONEXEC_TARGET_PROFILE_NAME) < 0) {
		return 2;
	}
	if (read_file_equals(APPARMOR_ATTR_EXEC_PATH,
			     "asterinas-aa-onexec-target\n") < 0) {
		return 3;
	}

	execl(EXEC_HELPER_PATH, EXEC_HELPER_PATH, ONEXEC_OUTPUT_PATH, NULL);
	return 4;
}

static int run_exec_transition_child(const char *profile_name,
				     const char *output_path,
				     const char *secure_output_path)
{
	if (write_text_file(APPARMOR_ATTR_CURRENT_PATH, profile_name) < 0) {
		return 1;
	}

	if (secure_output_path != NULL) {
		execl(EXEC_HELPER_PATH, EXEC_HELPER_PATH, output_path,
		      secure_output_path, NULL);
	} else {
		execl(EXEC_HELPER_PATH, EXEC_HELPER_PATH, output_path, NULL);
	}
	return 2;
}

static int run_denied_exec_transition_child(const char *profile_name)
{
	if (write_text_file(APPARMOR_ATTR_CURRENT_PATH, profile_name) < 0) {
		return 1;
	}

	execl(EXEC_HELPER_PATH, EXEC_HELPER_PATH, EXEC_DENIED_OUTPUT_PATH,
	      NULL);
	if (errno != EACCES) {
		return 2;
	}

	return 0;
}

FN_SETUP(register_securityfs_cleanup)
{
	atexit(cleanup_securityfs_mount);
}
END_SETUP()

FN_TEST(procfs_visibility_follows_lsm_selection)
{
	bool expect_apparmor = expect_apparmor_enabled();

	if (expect_apparmor) {
		TEST_SUCC(stat_file_type(APPARMOR_PROC_DIR_PATH, S_IFDIR));
		TEST_SUCC(stat_file_type(APPARMOR_PROC_PROFILES_PATH, S_IFREG));
		TEST_SUCC(read_file_equals(APPARMOR_PROC_CURRENT_PATH,
					   "unconfined\n"));
	} else {
		TEST_ERRNO(stat_file_type(APPARMOR_PROC_DIR_PATH, S_IFDIR),
			   ENOENT);
		TEST_ERRNO(open(APPARMOR_PROC_PROFILES_PATH, O_RDONLY), ENOENT);
		TEST_ERRNO(open(APPARMOR_PROC_CURRENT_PATH, O_RDONLY), ENOENT);
	}
}
END_TEST()

FN_TEST(task_attr_files_follow_lsm_selection)
{
	bool expect_apparmor = expect_apparmor_enabled();

	if (expect_apparmor) {
		TEST_SUCC(read_file_equals(APPARMOR_ATTR_CURRENT_PATH,
					   "unconfined\n"));
		TEST_SUCC(read_file_equals(APPARMOR_ATTR_EXEC_PATH, ""));
		TEST_SUCC(read_file_equals(APPARMOR_ATTR_PREV_PATH, ""));
	} else {
		TEST_RES(read_file_errno(APPARMOR_ATTR_CURRENT_PATH),
			 _ret == ENOENT);
		TEST_RES(read_file_errno(APPARMOR_ATTR_EXEC_PATH),
			 _ret == ENOENT);
		TEST_RES(read_file_errno(APPARMOR_ATTR_PREV_PATH),
			 _ret == ENOENT);
	}
}
END_TEST()

FN_TEST(securityfs_visibility_follows_lsm_selection)
{
	bool expect_apparmor = expect_apparmor_enabled();

	TEST_SUCC(mount_securityfs());

	if (expect_apparmor) {
		TEST_SUCC(stat_file_type(SECURITYFS_APPARMOR_DIR, S_IFDIR));
		TEST_SUCC(
			stat_file_type(SECURITYFS_APPARMOR_PROFILES, S_IFREG));
		TEST_SUCC(read_file_contains(
			SECURITYFS_APPARMOR_ABI,
			"asterinas-apparmor-linux-filedfa-v1"));
	} else {
		TEST_ERRNO(stat_file_type(SECURITYFS_APPARMOR_DIR, S_IFDIR),
			   ENOENT);
		TEST_ERRNO(open(SECURITYFS_APPARMOR_PROFILES, O_RDONLY),
			   ENOENT);
	}

	cleanup_securityfs_mount();
}
END_TEST()

FN_TEST(policy_modes_features_and_lifecycle)
{
	static const char enforce_policy[] =
		"profile asterinas-aa-mode-enforce enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"deny /tmp/aa-policy-mode r audit\n";
	static const char complain_policy[] =
		"profile asterinas-aa-mode-complain complain\n"
		"allow capability all\n"
		"allow /** all\n"
		"deny /tmp/aa-policy-mode r audit\n";
	static const char complain_implicit_policy[] =
		"profile asterinas-aa-mode-complain-implicit complain\n"
		"allow capability all\n";
	static const char replace_deny_policy[] =
		"profile asterinas-aa-replace-remove enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"deny /tmp/aa-policy-mode r\n";
	static const char replace_allow_policy[] =
		"profile asterinas-aa-replace-remove enforce\n"
		"allow capability all\n"
		"allow /** all\n";
	static const char remove_policy[] =
		"remove asterinas-aa-replace-remove\n";
	pid_t child;
	bool expect_apparmor = expect_apparmor_enabled();

	SKIP_TEST_IF(!expect_apparmor);

	TEST_SUCC(create_text_file(POLICY_MODE_PATH, "mode"));
	TEST_SUCC(mount_securityfs());
	TEST_SUCC(read_file_contains(SECURITYFS_APPARMOR_ABI,
				     "policy_abi=linux-v5-v9-subset"));
	TEST_SUCC(read_file_contains(SECURITYFS_APPARMOR_ABI, "complain=yes"));
	TEST_SUCC(read_file_contains(SECURITYFS_APPARMOR_PERMSTABLE,
				     "allow deny audit quiet xindex"));
	TEST_SUCC(read_file_contains(SECURITYFS_APPARMOR_FILE_MASK,
				     "delete rename setattr"));
	TEST_ERRNO(stat_file_type(SECURITYFS_APPARMOR_DOMAIN_STACK, S_IFREG),
		   ENOENT);
	cleanup_securityfs_mount();

	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH, enforce_policy));
	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_read_policy_mode_child(POLICY_MODE_PROFILE_ENFORCE,
						 true));
	}
	TEST_SUCC(wait_for_child_success(child));

	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH,
				  complain_implicit_policy));
	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_read_policy_mode_child(
			POLICY_MODE_PROFILE_COMPLAIN_IMPLICIT, false));
	}
	TEST_SUCC(wait_for_child_success(child));

	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH, complain_policy));
	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_read_policy_mode_child(POLICY_MODE_PROFILE_COMPLAIN,
						 true));
	}
	TEST_SUCC(wait_for_child_success(child));

	TEST_SUCC(
		write_text_file(APPARMOR_PROC_LOAD_PATH, replace_deny_policy));
	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_read_policy_mode_child(POLICY_REPLACE_PROFILE_NAME,
						 true));
	}
	TEST_SUCC(wait_for_child_success(child));

	TEST_SUCC(
		write_text_file(APPARMOR_PROC_LOAD_PATH, replace_allow_policy));
	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_read_policy_mode_child(POLICY_REPLACE_PROFILE_NAME,
						 false));
	}
	TEST_SUCC(wait_for_child_success(child));

	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH, remove_policy));
	TEST_SUCC(read_file_not_contains(APPARMOR_PROC_PROFILES_PATH,
					 POLICY_REPLACE_PROFILE_NAME));

	unlink(POLICY_MODE_PATH);
}
END_TEST()

FN_TEST(profile_change_and_exec_transition)
{
	static const char change_source_policy[] =
		"profile asterinas-aa-change-source enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"allow change_profile asterinas-aa-change-target\n"
		"allow change_onexec asterinas-aa-onexec-target\n";
	static const char change_target_policy[] =
		"profile asterinas-aa-change-target enforce\n"
		"allow capability all\n"
		"allow /** all\n";
	static const char onexec_target_policy[] =
		"profile asterinas-aa-onexec-target enforce\n"
		"allow capability all\n"
		"allow /** all\n";
	static const char exec_unsafe_source_policy[] =
		"profile asterinas-aa-unsafe-source enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"allow /test/security/lsm/apparmor_exec_helper x px:asterinas-aa-exec-target\n";
	static const char exec_source_policy[] =
		"profile asterinas-aa-exec-source enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"allow /test/security/lsm/apparmor_exec_helper x Px:asterinas-aa-exec-target\n";
	static const char exec_target_policy[] =
		"profile asterinas-aa-exec-target enforce\n"
		"allow capability all\n"
		"allow /** all\n";
	static const char ix_source_policy[] =
		"profile asterinas-aa-ix-source enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"allow /test/security/lsm/apparmor_exec_helper x ix\n";
	static const char ux_unsafe_source_policy[] =
		"profile asterinas-aa-ux-unsafe-source enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"allow /test/security/lsm/apparmor_exec_helper x ux\n";
	static const char ux_source_policy[] =
		"profile asterinas-aa-ux-source enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"allow /test/security/lsm/apparmor_exec_helper x Ux\n";
	static const char child_source_policy[] =
		"profile asterinas-aa-child-source enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"allow /test/security/lsm/apparmor_exec_helper x Cx:" EXEC_CHILD_TARGET_PROFILE_NAME
		"\n";
	static const char child_target_policy[] =
		"profile " EXEC_CHILD_TARGET_PROFILE_NAME " enforce\n"
		"allow capability all\n"
		"allow /** all\n";
	static const char cx_source_policy[] =
		"profile asterinas-aa-cx-source enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"allow /test/security/lsm/apparmor_exec_helper x cx:" EXEC_CX_TARGET_PROFILE_NAME
		"\n";
	static const char cx_target_policy[] =
		"profile " EXEC_CX_TARGET_PROFILE_NAME " enforce\n"
		"allow capability all\n"
		"allow /** all\n";
	static const char bad_child_source_policy[] =
		"profile asterinas-aa-bad-child-source enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"allow /test/security/lsm/apparmor_exec_helper x cx:"
		"asterinas-aa-exec-target\n";
	pid_t child;
	bool expect_apparmor = expect_apparmor_enabled();

	SKIP_TEST_IF(!expect_apparmor);

	unlink(ONEXEC_OUTPUT_PATH);
	unlink(EXEC_UNSAFE_OUTPUT_PATH);
	unlink(EXEC_UNSAFE_SECURE_PATH);
	unlink(EXEC_TRANSITION_OUTPUT_PATH);
	unlink(EXEC_TRANSITION_SECURE_PATH);
	unlink(EXEC_IX_OUTPUT_PATH);
	unlink(EXEC_IX_SECURE_PATH);
	unlink(EXEC_UX_UNSAFE_OUTPUT_PATH);
	unlink(EXEC_UX_UNSAFE_SECURE_PATH);
	unlink(EXEC_UX_OUTPUT_PATH);
	unlink(EXEC_UX_SECURE_PATH);
	unlink(EXEC_CHILD_OUTPUT_PATH);
	unlink(EXEC_CHILD_SECURE_PATH);
	unlink(EXEC_CX_OUTPUT_PATH);
	unlink(EXEC_CX_SECURE_PATH);
	unlink(EXEC_DENIED_OUTPUT_PATH);

	TEST_SUCC(
		write_text_file(APPARMOR_PROC_LOAD_PATH, change_target_policy));
	TEST_SUCC(
		write_text_file(APPARMOR_PROC_LOAD_PATH, onexec_target_policy));
	TEST_SUCC(
		write_text_file(APPARMOR_PROC_LOAD_PATH, change_source_policy));
	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH, exec_target_policy));
	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH,
				  exec_unsafe_source_policy));
	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH, exec_source_policy));
	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH, ix_source_policy));
	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH,
				  ux_unsafe_source_policy));
	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH, ux_source_policy));
	TEST_SUCC(
		write_text_file(APPARMOR_PROC_LOAD_PATH, child_target_policy));
	TEST_SUCC(
		write_text_file(APPARMOR_PROC_LOAD_PATH, child_source_policy));
	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH, cx_target_policy));
	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH, cx_source_policy));
	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH,
				  bad_child_source_policy));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_change_profile_child());
	}
	TEST_SUCC(wait_for_child_success(child));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_onexec_child());
	}
	TEST_SUCC(wait_for_child_success(child));
	TEST_SUCC(read_file_equals(ONEXEC_OUTPUT_PATH,
				   "asterinas-aa-onexec-target\n"));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_exec_transition_child(EXEC_UNSAFE_SOURCE_PROFILE_NAME,
						EXEC_UNSAFE_OUTPUT_PATH,
						EXEC_UNSAFE_SECURE_PATH));
	}
	TEST_SUCC(wait_for_child_success(child));
	TEST_SUCC(read_file_equals(EXEC_UNSAFE_OUTPUT_PATH,
				   "asterinas-aa-exec-target\n"));
	TEST_SUCC(read_file_equals(EXEC_UNSAFE_SECURE_PATH, "0\n"));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_exec_transition_child(EXEC_SOURCE_PROFILE_NAME,
						EXEC_TRANSITION_OUTPUT_PATH,
						EXEC_TRANSITION_SECURE_PATH));
	}
	TEST_SUCC(wait_for_child_success(child));
	TEST_SUCC(read_file_equals(EXEC_TRANSITION_OUTPUT_PATH,
				   "asterinas-aa-exec-target\n"));
	TEST_SUCC(read_file_equals(EXEC_TRANSITION_SECURE_PATH, "1\n"));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_exec_transition_child(EXEC_IX_SOURCE_PROFILE_NAME,
						EXEC_IX_OUTPUT_PATH,
						EXEC_IX_SECURE_PATH));
	}
	TEST_SUCC(wait_for_child_success(child));
	TEST_SUCC(read_file_equals(EXEC_IX_OUTPUT_PATH,
				   "asterinas-aa-ix-source\n"));
	TEST_SUCC(read_file_equals(EXEC_IX_SECURE_PATH, "0\n"));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_exec_transition_child(
			EXEC_UX_UNSAFE_SOURCE_PROFILE_NAME,
			EXEC_UX_UNSAFE_OUTPUT_PATH,
			EXEC_UX_UNSAFE_SECURE_PATH));
	}
	TEST_SUCC(wait_for_child_success(child));
	TEST_SUCC(read_file_equals(EXEC_UX_UNSAFE_OUTPUT_PATH, "unconfined\n"));
	TEST_SUCC(read_file_equals(EXEC_UX_UNSAFE_SECURE_PATH, "0\n"));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_exec_transition_child(EXEC_UX_SOURCE_PROFILE_NAME,
						EXEC_UX_OUTPUT_PATH,
						EXEC_UX_SECURE_PATH));
	}
	TEST_SUCC(wait_for_child_success(child));
	TEST_SUCC(read_file_equals(EXEC_UX_OUTPUT_PATH, "unconfined\n"));
	TEST_SUCC(read_file_equals(EXEC_UX_SECURE_PATH, "1\n"));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_exec_transition_child(EXEC_CHILD_SOURCE_PROFILE_NAME,
						EXEC_CHILD_OUTPUT_PATH,
						EXEC_CHILD_SECURE_PATH));
	}
	TEST_SUCC(wait_for_child_success(child));
	TEST_SUCC(read_file_equals(EXEC_CHILD_OUTPUT_PATH,
				   EXEC_CHILD_TARGET_PROFILE_NAME "\n"));
	TEST_SUCC(read_file_equals(EXEC_CHILD_SECURE_PATH, "1\n"));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_exec_transition_child(EXEC_CX_SOURCE_PROFILE_NAME,
						EXEC_CX_OUTPUT_PATH,
						EXEC_CX_SECURE_PATH));
	}
	TEST_SUCC(wait_for_child_success(child));
	TEST_SUCC(read_file_equals(EXEC_CX_OUTPUT_PATH,
				   EXEC_CX_TARGET_PROFILE_NAME "\n"));
	TEST_SUCC(read_file_equals(EXEC_CX_SECURE_PATH, "0\n"));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_denied_exec_transition_child(
			EXEC_BAD_CHILD_SOURCE_PROFILE_NAME));
	}
	TEST_SUCC(wait_for_child_success(child));
	TEST_ERRNO(stat_file_type(EXEC_DENIED_OUTPUT_PATH, S_IFREG), ENOENT);

	unlink(ONEXEC_OUTPUT_PATH);
	unlink(EXEC_UNSAFE_OUTPUT_PATH);
	unlink(EXEC_UNSAFE_SECURE_PATH);
	unlink(EXEC_TRANSITION_OUTPUT_PATH);
	unlink(EXEC_TRANSITION_SECURE_PATH);
	unlink(EXEC_IX_OUTPUT_PATH);
	unlink(EXEC_IX_SECURE_PATH);
	unlink(EXEC_UX_UNSAFE_OUTPUT_PATH);
	unlink(EXEC_UX_UNSAFE_SECURE_PATH);
	unlink(EXEC_UX_OUTPUT_PATH);
	unlink(EXEC_UX_SECURE_PATH);
	unlink(EXEC_CHILD_OUTPUT_PATH);
	unlink(EXEC_CHILD_SECURE_PATH);
	unlink(EXEC_CX_OUTPUT_PATH);
	unlink(EXEC_CX_SECURE_PATH);
	unlink(EXEC_DENIED_OUTPUT_PATH);
}
END_TEST()

FN_TEST(file_mediation_revalidates_runtime_operations)
{
	static const char policy[] =
		"profile asterinas-aa-file-hooks enforce\n"
		"allow capability all\n"
		"allow /** all\n"
		"deny /tmp/aa-file-preopened r\n"
		"deny /tmp/aa-file-access r\n"
		"deny /tmp create\n"
		"deny /tmp/aa-file-getattr r\n"
		"deny /tmp/aa-file-readlink r\n"
		"deny /tmp/aa-file-receive r\n"
		"deny /tmp/aa-file-mmap mmap\n"
		"deny /tmp/aa-file-rename-target delete\n"
		"deny /tmp/aa-file-exchange-target rename\n";
	pid_t child;
	bool expect_apparmor = expect_apparmor_enabled();

	SKIP_TEST_IF(!expect_apparmor);

	unlink(FILE_HOOK_PREOPENED_PATH);
	unlink(FILE_HOOK_ACCESS_PATH);
	unlink(FILE_HOOK_MMAP_PATH);
	unlink(FILE_HOOK_GETATTR_PATH);
	unlink(FILE_HOOK_READLINK_PATH);
	unlink(FILE_HOOK_RECEIVE_PATH);
	unlink(FILE_HOOK_SYMLINK_TARGET);
	unlink(FILE_HOOK_RENAME_SOURCE_PATH);
	unlink(FILE_HOOK_RENAME_TARGET_PATH);
	unlink(FILE_HOOK_EXCHANGE_SOURCE_PATH);
	unlink(FILE_HOOK_EXCHANGE_TARGET_PATH);

	TEST_SUCC(create_text_file(FILE_HOOK_PREOPENED_PATH, "preopened"));
	TEST_SUCC(create_text_file(FILE_HOOK_ACCESS_PATH, "access"));
	TEST_SUCC(create_text_file(FILE_HOOK_MMAP_PATH, "mmap"));
	TEST_SUCC(create_text_file(FILE_HOOK_GETATTR_PATH, "getattr"));
	TEST_SUCC(create_text_file(FILE_HOOK_RECEIVE_PATH, "receive"));
	TEST_SUCC(create_text_file(FILE_HOOK_SYMLINK_TARGET, "target"));
	TEST_SUCC(create_text_file(FILE_HOOK_RENAME_SOURCE_PATH, "source"));
	TEST_SUCC(create_text_file(FILE_HOOK_RENAME_TARGET_PATH, "target"));
	TEST_SUCC(create_text_file(FILE_HOOK_EXCHANGE_SOURCE_PATH, "source"));
	TEST_SUCC(create_text_file(FILE_HOOK_EXCHANGE_TARGET_PATH, "target"));
	TEST_SUCC(symlink(FILE_HOOK_SYMLINK_TARGET, FILE_HOOK_READLINK_PATH));
	TEST_SUCC(write_text_file(APPARMOR_PROC_LOAD_PATH, policy));

	child = fork();
	TEST(child, 0, _ret >= 0);
	if (child == 0) {
		_exit(run_file_hook_child());
	}
	TEST_SUCC(wait_for_child_success(child));

	unlink(FILE_HOOK_PREOPENED_PATH);
	unlink(FILE_HOOK_ACCESS_PATH);
	unlink(FILE_HOOK_MMAP_PATH);
	unlink(FILE_HOOK_GETATTR_PATH);
	unlink(FILE_HOOK_READLINK_PATH);
	unlink(FILE_HOOK_RECEIVE_PATH);
	unlink(FILE_HOOK_SYMLINK_TARGET);
	unlink(FILE_HOOK_RENAME_SOURCE_PATH);
	unlink(FILE_HOOK_RENAME_TARGET_PATH);
	unlink(FILE_HOOK_EXCHANGE_SOURCE_PATH);
	unlink(FILE_HOOK_EXCHANGE_TARGET_PATH);
}
END_TEST()
