/* SPDX-License-Identifier: MPL-2.0 */

#ifndef NOTIFICATION_COMMON_H
#define NOTIFICATION_COMMON_H

#include "seccomp_common.h"
#include <sys/ioctl.h>

static inline int notification_filter(uint32_t action, unsigned int flags)
{
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, action),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = { .len = 4, .filter = code };
	return syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, flags, &prog);
}

static inline int listener_flags(unsigned int flags)
{
	no_new_privs();
	return CHECK(
		notification_filter(SECCOMP_RET_USER_NOTIF | 0x1234,
				    SECCOMP_FILTER_FLAG_NEW_LISTENER | flags));
}

static inline int listener(void)
{
	return listener_flags(0);
}

static inline long notify_getppid(void)
{
	return syscall(SYS_getppid, 0x1122334455667788UL, 2UL, 3UL, 4UL, 5UL,
		       0x8877665544332211UL);
}

static inline pid_t notifying_child_logged(int fd, long expected,
					   int expected_errno, int log_count)
{
	pid_t child = CHECK(fork());
	if (child)
		return child;
	CHECK(close(fd));
	alarm(10);
	if (log_count >= 0) {
		printf("LOG_EXPECT pid=%d syscall=%d action=%#x count=%d\n",
		       getpid(), SYS_getppid, SECCOMP_RET_USER_NOTIF,
		       log_count);
		fflush(stdout);
	}
	errno = 0;
	long result = notify_getppid();
	int error = errno;
	if (result != expected || error != expected_errno)
		fprintf(stderr,
			"notification response: got %ld/%d, expected %ld/%d\n",
			result, error, expected, expected_errno);
	CHECK_WITH(result, _ret == expected && error == expected_errno);
	_exit(0);
}

static inline pid_t notifying_child(int fd, long expected, int expected_errno)
{
	return notifying_child_logged(fd, expected, expected_errno, -1);
}

static inline void reap(pid_t child)
{
	int status;
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	if (!WIFEXITED(status) || WEXITSTATUS(status) != 0)
		fprintf(stderr,
			"child %d: wait status=%#x, exit=%d, signal=%d\n",
			child, status,
			WIFEXITED(status) ? WEXITSTATUS(status) : -1,
			WIFSIGNALED(status) ? WTERMSIG(status) : 0);
	CHECK_WITH(status, WIFEXITED(_ret) && WEXITSTATUS(_ret) == 0);
}

static inline struct seccomp_notif receive(int fd, pid_t child)
{
	struct seccomp_notif request = { 0 };
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, &request));
	CHECK_WITH(request.pid, _ret == child);
	CHECK_WITH(request.flags, _ret == 0);
	CHECK_WITH(request.data.nr, _ret == SYS_getppid);
	CHECK_WITH(request.data.arch, _ret == NATIVE_ARCH);
	CHECK_WITH(request.data.instruction_pointer, _ret != 0);
	CHECK_WITH(request.data.args[0], _ret == 0x1122334455667788UL);
	CHECK_WITH(request.data.args[1], _ret == 2);
	CHECK_WITH(request.data.args[5], _ret == 0x8877665544332211UL);
	CHECK(ioctl(fd, SECCOMP_IOCTL_NOTIF_ID_VALID, &request.id));
	CHECK(ioctl(fd, _IOR('!', 2, uint64_t), &request.id));
	return request;
}

#endif
