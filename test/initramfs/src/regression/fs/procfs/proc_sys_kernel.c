// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/utsname.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/capability.h"
#include "../../common/test.h"

#define ARRAY_SIZE(array) (sizeof(array) / sizeof((array)[0]))
#define UTS_FIELD_LEN 65

static void read_file_checked(const char *path, char *buf, size_t buf_len)
{
	int fd = CHECK(open(path, O_RDONLY));
	ssize_t read_len = CHECK(read(fd, buf, buf_len - 1));

	buf[read_len] = '\0';
	CHECK(close(fd));
}

static ssize_t write_file_len(const char *path, const char *buf, size_t buf_len)
{
	int fd = open(path, O_WRONLY);

	if (fd < 0) {
		return -1;
	}

	ssize_t written = write(fd, buf, buf_len);
	int saved_errno = errno;

	if (close(fd) < 0 && written >= 0) {
		return -1;
	}

	errno = saved_errno;
	return written;
}

static ssize_t write_file(const char *path, const char *buf)
{
	return write_file_len(path, buf, strlen(buf));
}

static int proc_field_matches(const char *path, const char *field)
{
	char expected[UTS_FIELD_LEN + 1];
	char actual[UTS_FIELD_LEN + 1];

	snprintf(expected, sizeof(expected), "%s\n", field);
	read_file_checked(path, actual, sizeof(actual));
	return strcmp(actual, expected);
}

static unsigned long long read_tainted(void)
{
	char value[32];

	read_file_checked("/proc/sys/kernel/tainted", value, sizeof(value));
	return strtoull(value, NULL, 10);
}

FN_TEST(proc_sys_kernel_uts_files_match_uname)
{
	struct utsname uts_name;

	TEST_SUCC(uname(&uts_name));

	TEST_RES(proc_field_matches("/proc/sys/kernel/hostname",
				    uts_name.nodename),
		 _ret == 0);
	TEST_RES(proc_field_matches("/proc/sys/kernel/domainname",
				    uts_name.domainname),
		 _ret == 0);
	TEST_RES(proc_field_matches("/proc/sys/kernel/osrelease",
				    uts_name.release),
		 _ret == 0);
	TEST_RES(proc_field_matches("/proc/sys/kernel/version",
				    uts_name.version),
		 _ret == 0);
}
END_TEST()

FN_TEST(proc_sys_kernel_uts_files_update_namespace)
{
	const char nul_name[] = { 'h', 'e', 'l', 'l', 'o', '\0',
				  'w', 'o', 'r', 'l', 'd' };
	struct utsname original;
	struct utsname updated;
	char expected_long_name[UTS_FIELD_LEN];
	char long_name[1000];

	TEST_SUCC(uname(&original));

	TEST_RES(write_file("/proc/sys/kernel/hostname", "proc-host\n"),
		 _ret == (ssize_t)strlen("proc-host\n"));
	TEST_SUCC(uname(&updated));
	TEST_RES(strcmp(updated.nodename, "proc-host"), _ret == 0);

	TEST_RES(write_file("/proc/sys/kernel/hostname", "localhost\n123"),
		 _ret == (ssize_t)strlen("localhost\n123"));
	TEST_SUCC(uname(&updated));
	TEST_RES(strcmp(updated.nodename, "localhost"), _ret == 0);

	memset(long_name, 'a', sizeof(long_name));
	memset(expected_long_name, 'a', UTS_FIELD_LEN - 1);
	expected_long_name[UTS_FIELD_LEN - 1] = '\0';
	TEST_RES(write_file_len("/proc/sys/kernel/hostname", long_name,
				sizeof(long_name)),
		 _ret == (ssize_t)sizeof(long_name));
	TEST_SUCC(uname(&updated));
	TEST_RES(strcmp(updated.nodename, expected_long_name), _ret == 0);

	TEST_RES(write_file_len("/proc/sys/kernel/hostname", nul_name,
				sizeof(nul_name)),
		 _ret == (ssize_t)sizeof(nul_name));
	TEST_SUCC(uname(&updated));
	TEST_RES(strcmp(updated.nodename, "hello"), _ret == 0);

	TEST_SUCC(sethostname("syscall-host", strlen("syscall-host")));
	TEST_SUCC(uname(&updated));
	TEST_RES(proc_field_matches("/proc/sys/kernel/hostname",
				    updated.nodename),
		 _ret == 0);

	TEST_RES(write_file("/proc/sys/kernel/domainname", "proc-domain\n"),
		 _ret == (ssize_t)strlen("proc-domain\n"));
	TEST_SUCC(uname(&updated));
	TEST_RES(strcmp(updated.domainname, "proc-domain"), _ret == 0);

	TEST_RES(write_file("/proc/sys/kernel/domainname", "localdomain\n123"),
		 _ret == (ssize_t)strlen("localdomain\n123"));
	TEST_SUCC(uname(&updated));
	TEST_RES(strcmp(updated.domainname, "localdomain"), _ret == 0);

	TEST_RES(write_file_len("/proc/sys/kernel/domainname", long_name,
				sizeof(long_name)),
		 _ret == (ssize_t)sizeof(long_name));
	TEST_SUCC(uname(&updated));
	TEST_RES(strcmp(updated.domainname, expected_long_name), _ret == 0);

	TEST_RES(write_file_len("/proc/sys/kernel/domainname", nul_name,
				sizeof(nul_name)),
		 _ret == (ssize_t)sizeof(nul_name));
	TEST_SUCC(uname(&updated));
	TEST_RES(strcmp(updated.domainname, "hello"), _ret == 0);

	TEST_SUCC(setdomainname("syscall-domain", strlen("syscall-domain")));
	TEST_SUCC(uname(&updated));
	TEST_RES(proc_field_matches("/proc/sys/kernel/domainname",
				    updated.domainname),
		 _ret == 0);

	TEST_SUCC(sethostname(original.nodename, strlen(original.nodename)));
	TEST_SUCC(setdomainname(original.domainname,
				strlen(original.domainname)));
}
END_TEST()

FN_TEST(proc_sys_kernel_tainted_tracks_written_bits)
{
	unsigned long long initial;
	pid_t pid;
	int status;

	initial = read_tainted();

	TEST_RES(write_file("/proc/sys/kernel/tainted", "1\n"),
		 _ret == (ssize_t)strlen("1\n"));
	TEST_RES(read_tainted(), _ret == (initial | 1ULL));

	TEST_RES(write_file("/proc/sys/kernel/tainted", "2\n"),
		 _ret == (ssize_t)strlen("2\n"));
	TEST_RES(read_tainted(), _ret == (initial | 3ULL));

	TEST_RES(write_file("/proc/sys/kernel/tainted", "0\n"),
		 _ret == (ssize_t)strlen("0\n"));
	TEST_RES(read_tainted(), _ret == (initial | 3ULL));

	TEST_RES(write_file("/proc/sys/kernel/tainted",
			    "1048576\n"), // 1 << 20
		 _ret == (ssize_t)strlen("1048576\n"));
	TEST_RES(read_tainted(), _ret == (initial | 3ULL));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		drop_capability(CAP_SYS_ADMIN);

		errno = 0;
		CHECK_WITH(write_file("/proc/sys/kernel/tainted", "4\n"),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(read_tainted(), _ret == (initial | 3ULL));

		_exit(0);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()
