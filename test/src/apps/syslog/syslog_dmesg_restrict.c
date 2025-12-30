// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#define SYSLOG_ACTION_READ_ALL 3
#define SYSLOG_ACTION_SIZE_BUFFER 10
#define DMESG_PATH "/proc/sys/kernel/dmesg_restrict"

static void drop_priv_or_die(void)
{
    if (geteuid() != 0)
        return;

    if (setgid(65534) != 0) {
        fprintf(stderr, "setgid failed: %s\n", strerror(errno));
        _exit(1);
    }
    if (setuid(65534) != 0) {
        fprintf(stderr, "setuid failed: %s\n", strerror(errno));
        _exit(1);
    }

    if (geteuid() == 0) {
        fprintf(stderr, "priv drop failed: still root\n");
        _exit(1);
    }
}


static int read_dmesg_restrict(void)
{
	FILE *f = fopen(DMESG_PATH, "re");
	if (!f) {
		perror("fopen dmesg_restrict");
		return -1;
	}
	int val = -1;
	if (fscanf(f, "%d", &val) != 1) {
		fprintf(stderr, "failed to parse dmesg_restrict\n");
		val = -1;
	}
	fclose(f);
	return val;
}

static int write_dmesg_restrict(int val)
{
	FILE *f = fopen(DMESG_PATH, "we");
	if (!f) {
		perror("fopen dmesg_restrict for write");
		return -1;
	}
	fprintf(f, "%d\n", val);
	int err = ferror(f);
	fclose(f);
	return err ? -1 : 0;
}

static int run_unpriv_checks(int expect_success)
{
	pid_t pid = fork();
	if (pid < 0) {
		perror("fork");
		return -1;
	}

	if (pid == 0) {
		if (geteuid() == 0) 
            drop_priv_or_die();

		errno = 0;
		long size = syscall(SYS_syslog, SYSLOG_ACTION_SIZE_BUFFER, 0, 0);
		int size_ok = (size >= 0);
		int size_errno = errno;

		errno = 0;
		char buf[16];
		long rd = syscall(SYS_syslog, SYSLOG_ACTION_READ_ALL, buf,
				  sizeof(buf));
		int read_ok = (rd >= 0);
		int read_errno = errno;

		if (expect_success) {
			if (!size_ok || !read_ok) {
				fprintf(stderr,
					"expected success: size_ok=%d errno=%d read_ok=%d errno=%d\n",
					size_ok, size_errno, read_ok,
					read_errno);
				_exit(1);
			}
		} else {
			if (size_ok || size_errno != EPERM || read_ok ||
			    read_errno != EPERM) {
				fprintf(stderr,
					"expected EPERM: size_ok=%d errno=%d read_ok=%d errno=%d\n",
					size_ok, size_errno, read_ok,
					read_errno);
				_exit(1);
			}
		}
		_exit(0);
	}

	int status = 0;
	if (waitpid(pid, &status, 0) < 0) {
		perror("waitpid");
		return -1;
	}
	if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
		return -1;
	}
	return 0;
}

int main(void)
{
	int orig = read_dmesg_restrict();
	if (orig < 0) {
		return 1;
	}

	if (write_dmesg_restrict(0) < 0) {
		return 1;
	}
	if (run_unpriv_checks(1) < 0) {
		return 1;
	}

	if (write_dmesg_restrict(1) < 0) {
		return 1;
	}
	if (run_unpriv_checks(0) < 0) {
		return 1;
	}

	if (orig != 0 && orig != 1) {
		orig = 0;
	}
	(void)write_dmesg_restrict(orig);
	return 0;
}

