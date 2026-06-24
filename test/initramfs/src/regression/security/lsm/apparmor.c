// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <ctype.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#define CMDLINE_BUFFER_SIZE 4096
#define FILE_BUFFER_SIZE 4096

static const char *CMDLINE_PATH = "/proc/cmdline";
static const char *APPARMOR_PROC_DIR_PATH = "/proc/sys/kernel/apparmor";
static const char *APPARMOR_PROC_PROFILES_PATH =
	"/proc/sys/kernel/apparmor/profiles";
static const char *APPARMOR_PROC_CURRENT_PATH =
	"/proc/sys/kernel/apparmor/current";
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

static bool securityfs_mounted;

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
		TEST_ERRNO(open(APPARMOR_PROC_PROFILES_PATH, O_RDONLY),
			   ENOENT);
		TEST_ERRNO(open(APPARMOR_PROC_CURRENT_PATH, O_RDONLY), ENOENT);
	}
}
END_TEST()

FN_TEST(task_attr_files_follow_lsm_selection)
{
	bool expect_apparmor = expect_apparmor_enabled();

	if (expect_apparmor) {
		TEST_SUCC(
			read_file_equals(APPARMOR_ATTR_CURRENT_PATH, "unconfined\n"));
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
		TEST_SUCC(stat_file_type(SECURITYFS_APPARMOR_PROFILES, S_IFREG));
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
