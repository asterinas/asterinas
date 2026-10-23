// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/capability.h"
#include <fcntl.h>
#include <linux/capability.h>
#include <stdbool.h>
#include <stdint.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#define APPARMOR_DIR "/sys/kernel/security/apparmor"
#define LOAD_PATH APPARMOR_DIR "/.load"
#define PROFILES_PATH APPARMOR_DIR "/profiles"
#define FIXTURE_DIR "/test/security/lsm/apparmor-fixtures/"
#define CMDLINE_BUFFER_SIZE 4096
#define MAX_POLICY_BLOB_SIZE (1024 * 1024)

struct fixture {
	unsigned char *data;
	size_t len;
};

static bool apparmor_selected(void)
{
	char cmdline[CMDLINE_BUFFER_SIZE];
	char lsm_param[CMDLINE_BUFFER_SIZE] = "";
	char security_param[CMDLINE_BUFFER_SIZE] = "";
	char *saveptr = NULL;
	bool has_lsm_param = false;
	bool has_security_param = false;
	int fd = CHECK(open("/proc/cmdline", O_RDONLY));
	ssize_t len = CHECK(read(fd, cmdline, sizeof(cmdline) - 1));

	CHECK(close(fd));
	cmdline[len] = '\0';
	for (char *token = strtok_r(cmdline, " \n", &saveptr); token;
	     token = strtok_r(NULL, " \n", &saveptr)) {
		if (strncmp(token, "lsm=", strlen("lsm=")) == 0) {
			CHECK_WITH(snprintf(lsm_param, sizeof(lsm_param), "%s",
					    token + strlen("lsm=")),
				   _ret >= 0 && _ret < (int)sizeof(lsm_param));
			has_lsm_param = true;
		} else if (strncmp(token, "security=", strlen("security=")) ==
			   0) {
			CHECK_WITH(snprintf(security_param,
					    sizeof(security_param), "%s",
					    token + strlen("security=")),
				   _ret >= 0 &&
					   _ret < (int)sizeof(security_param));
			has_security_param = true;
		}
	}

	if (has_lsm_param) {
		char *module_saveptr = NULL;
		for (char *module = strtok_r(lsm_param, ",", &module_saveptr);
		     module; module = strtok_r(NULL, ",", &module_saveptr)) {
			if (strcmp(module, "apparmor") == 0) {
				return true;
			}
		}
		return false;
	}

	return has_security_param && strcmp(security_param, "apparmor") == 0;
}

static struct fixture read_fixture(const char *name)
{
	char path[256];
	struct stat statbuf;
	struct fixture fixture;
	ssize_t total = 0;

	CHECK_WITH(snprintf(path, sizeof(path), "%s%s", FIXTURE_DIR, name),
		   _ret > 0 && (size_t)_ret < sizeof(path));
	CHECK(stat(path, &statbuf));
	fixture.len = statbuf.st_size;
	fixture.data = CHECK_WITH(malloc(fixture.len), _ret != NULL);
	int fd = CHECK(open(path, O_RDONLY));
	while ((size_t)total < fixture.len) {
		total += CHECK_WITH(read(fd, fixture.data + total,
					 fixture.len - total),
				    _ret > 0);
	}
	CHECK(close(fd));
	return fixture;
}

static ssize_t load_data_at(const void *data, size_t len, off_t offset)
{
	int fd = open(LOAD_PATH, O_WRONLY);
	if (fd < 0) {
		return -1;
	}

	ssize_t result = pwrite(fd, data, len, offset);
	int saved_errno = errno;
	close(fd);
	errno = saved_errno;
	return result;
}

static ssize_t load_data(const void *data, size_t len)
{
	return load_data_at(data, len, 0);
}

static ssize_t read_profiles(char *output, size_t capacity)
{
	int fd = open(PROFILES_PATH, O_RDONLY);
	if (fd < 0) {
		return -1;
	}

	ssize_t result = read(fd, output, capacity - 1);
	int saved_errno = errno;
	if (result >= 0) {
		output[result] = '\0';
	}
	close(fd);
	errno = saved_errno;
	return result;
}

static void become_nobody_with_only_mac_admin(void)
{
	struct __user_cap_header_struct header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
		.pid = 0,
	};
	struct __user_cap_data_struct data[2] = {};
	unsigned int index = CAP_MAC_ADMIN / 32;
	uint32_t mask = 1U << (CAP_MAC_ADMIN % 32);

	CHECK(prctl(PR_SET_KEEPCAPS, 1));
	CHECK(setuid(65534));
	data[index].effective = mask;
	data[index].permitted = mask;
	CHECK(syscall(SYS_capset, &header, data));
	CHECK(syscall(SYS_capget, &header, data));
	CHECK_WITH(geteuid(), _ret == 65534);
	CHECK_WITH(data[0].effective, _ret == 0);
	CHECK_WITH(data[1].effective, _ret == mask);
}

#define TEST_PROFILES(expected)                                            \
	do {                                                               \
		char _profiles[512];                                       \
		TEST_RES(read_profiles(_profiles, sizeof(_profiles)),      \
			 _ret >= 0 && strcmp(_profiles, (expected)) == 0); \
	} while (0)

FN_TEST(securityfs_visibility_follows_selection)
{
	struct stat statbuf;

	TEST_RES(stat("/sys/kernel/security", &statbuf),
		 S_ISDIR(statbuf.st_mode));
	if (apparmor_selected()) {
		TEST_RES(stat(APPARMOR_DIR, &statbuf),
			 S_ISDIR(statbuf.st_mode));
	} else {
		TEST_ERRNO(stat(APPARMOR_DIR, &statbuf), ENOENT);
	}
}
END_TEST()

FN_TEST(policy_load_and_query_interface)
{
	struct stat statbuf;

	SKIP_TEST_IF(!apparmor_selected());

	TEST_RES(stat(APPARMOR_DIR, &statbuf),
		 S_ISDIR(statbuf.st_mode) && (statbuf.st_mode & 0777) == 0555);
	TEST_RES(stat(LOAD_PATH, &statbuf), (statbuf.st_mode & 0777) == 0666);
	TEST_RES(stat(PROFILES_PATH, &statbuf),
		 (statbuf.st_mode & 0777) == 0444);
	TEST_ERRNO(stat(APPARMOR_DIR "/.replace", &statbuf), ENOENT);
	TEST_ERRNO(stat(APPARMOR_DIR "/.remove", &statbuf), ENOENT);
	TEST_PROFILES("");
	TEST_ERRNO(load_data("", 0), EINVAL);

	int load_fd = TEST_SUCC(open(LOAD_PATH, O_WRONLY));
	char byte;
	TEST_ERRNO(read(load_fd, &byte, 1), EBADF);
	TEST_SUCC(close(load_fd));
	int profiles_fd = TEST_SUCC(open(PROFILES_PATH, O_RDONLY));
	TEST_ERRNO(write(profiles_fd, "x", 1), EBADF);
	TEST_SUCC(close(profiles_fd));

	struct fixture observe = read_fixture("observe.bin");
	TEST_RES(observe.len, _ret == 118);

	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		int child_load = CHECK(open(LOAD_PATH, O_WRONLY));
		int child_profiles = CHECK(open(PROFILES_PATH, O_RDONLY));
		drop_capability(CAP_MAC_ADMIN);
		CHECK_WITH(write(child_load, observe.data, observe.len),
			   _ret < 0 && errno == EACCES);
		CHECK_WITH(pwrite(child_load, observe.data, observe.len, 1),
			   _ret < 0 && errno == EACCES);
		CHECK_WITH(read(child_profiles, &byte, 1),
			   _ret < 0 && errno == EACCES);
		CHECK(close(child_profiles));
		CHECK(close(child_load));
		_exit(EXIT_SUCCESS);
	}
	int status;
	TEST_RES(waitpid(child, &status, 0),
		 _ret == child && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);

	child = TEST_SUCC(fork());
	if (child == 0) {
		char profiles[512];

		become_nobody_with_only_mac_admin();
		CHECK_WITH(load_data(observe.data, observe.len),
			   _ret == (ssize_t)observe.len);
		CHECK_WITH(read_profiles(profiles, sizeof(profiles)), _ret > 0);
		CHECK_WITH(strcmp(profiles, "observe (complain)\n"), _ret == 0);
		_exit(EXIT_SUCCESS);
	}
	TEST_RES(waitpid(child, &status, 0),
		 _ret == child && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_PROFILES("observe (complain)\n");

	TEST_ERRNO(load_data_at(observe.data, observe.len, 1), ESPIPE);
	TEST_ERRNO(load_data(observe.data, observe.len), EEXIST);
	TEST_ERRNO(load_data(observe.data, observe.len - 1), EPROTO);

	unsigned char *multiple =
		TEST_RES(malloc(observe.len * 2), _ret != NULL);
	memcpy(multiple, observe.data, observe.len);
	memcpy(multiple + observe.len, observe.data, observe.len);
	TEST_ERRNO(load_data(multiple, observe.len * 2), EOPNOTSUPP);
	free(multiple);

	unsigned char *oversize =
		TEST_RES(malloc(MAX_POLICY_BLOB_SIZE + 1), _ret != NULL);
	memset(oversize, 0, MAX_POLICY_BLOB_SIZE + 1);
	TEST_ERRNO(load_data(oversize, MAX_POLICY_BLOB_SIZE + 1), E2BIG);
	free(oversize);

	load_fd = TEST_SUCC(open(LOAD_PATH, O_WRONLY));
	TEST_ERRNO(syscall(SYS_write, load_fd, (void *)(uintptr_t)-1, 1),
		   EFAULT);
	TEST_SUCC(close(load_fd));
	TEST_PROFILES("observe (complain)\n");

	free(observe.data);
}
END_TEST()
